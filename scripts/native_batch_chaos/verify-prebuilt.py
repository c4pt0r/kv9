"""Read-only candidate/image/source checks; optionally copy only retained client evidence."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def output(argv):
    return subprocess.check_output(argv, text=True, timeout=30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--offline', action='store_true', help='Check retained bytes only; never authorize a fixture')
    parser.add_argument('--copy-workload', type=Path)
    parser.add_argument('--copy-native', type=Path)
    args = parser.parse_args()
    p = json.loads(args.plan.read_text())
    if p.get('plan_ready'):
        frozen_path = Path(p['ready_plan_path'])
        assert sha(frozen_path) == p['ready_plan_sha256'], 'frozen readiness plan changed'
        frozen = json.loads(frozen_path.read_text())
        for key, value in frozen.items():
            if key not in ('fixture_started', 'fixture_launch_authorized'):
                assert p[key] == value, ('frozen plan input changed', key)
    assert p['build_complete'] and p['source_unchanged'] and p['prepared_image']['complete']
    assert p['worktree'] == str(Path(__file__).resolve().parents[2]), 'plan must bind this repository for a new run'
    for filename, expected in p.get('overlay_inputs', {}).items():
        assert sha(filename) == expected, filename
    for filename, target in p.get('overlay_helper_links', {}).items():
        assert Path(filename).is_symlink() and str(Path(filename).resolve()) == target, filename
    tree = Path(p['worktree'])
    assert output(['git', '-C', str(tree), 'rev-parse', 'HEAD']).strip() == p['revision']
    assert not output(['git', '-C', str(tree), 'status', '--porcelain'])
    for name, expected in p['source'].items():
        assert sha(tree / name) == expected, name
    for key in ('production_binary', 'pressure_binary'):
        assert sha(p[key]['path']) == p[key]['sha256']
    build = args.plan.parent / 'persistent-build'
    client = json.loads((build / 'build.json').read_text())
    inventory = json.loads((build / 'sources.json').read_text())
    assert client == p['correctness_client_build'] and client['dirty'] is False
    assert client['revision'] == p['revision'] and client['source_tree_sha256'] == p['source_tree_sha256']
    assert inventory['sources'] == p['source'] and sha(build / 'kv9-workload') == client['binary_sha256']
    for filename, target in ((args.plan.parent / 'server-cargo.stdout', 'kv9'), (build / 'cargo.jsonl', 'kv9-workload')):
        rows = [json.loads(line) for line in filename.read_text().splitlines()]
        for name in (target, 'kv9_engine', 'kv9_raft', 'kv9_server'):
            found = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == name]
            assert len(found) == 1 and found[0]['features'] == [], name
    native = args.plan.parent / 'native-build'
    native_client = json.loads((native / 'build.json').read_text())
    assert native_client == p['native_client_build'] and native_client['dirty'] is False
    assert native_client['revision'] == p['revision'] and native_client['source_tree_sha256'] == p['source_tree_sha256']
    assert json.loads((native / 'sources.json').read_text())['sources'] == p['source']
    assert sha(native / 'kv9-batch-workload') == native_client['binary_sha256']
    for folder, target in ((build, 'kv9-workload'), (native, 'kv9-batch-workload')):
        rows = [json.loads(line) for line in (folder / 'cargo.jsonl').read_text().splitlines()]
        for name in (target, 'kv9_engine', 'kv9_raft', 'kv9_server'):
            found = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == name]
            assert len(found) == 1 and found[0]['features'] == [] and found[0]['profile']['test'] is False, name
        assert len([r for r in rows if r.get('reason') == 'build-finished']) == 1
        assert rows[-1] == dict(reason='build-finished', success=True)
    if args.offline:
        assert not args.copy_workload and not args.copy_native
        print(json.dumps(dict(complete=True, offline=True, revision=p['revision'])))
        return
    for name, expected in p['context_files'].items():
        assert sha(Path(p['context']) / name) == expected, name
    image = json.loads(output(['docker', 'image', 'inspect', p['image']]))[0]
    assert image['Id'] == p['prepared_image']['docker_image_id']
    nodes = sorted(output([p['kind'], 'get', 'nodes', '--name', p['kind_cluster']]).splitlines())
    kube = ['kubectl', '--kubeconfig', p['kubeconfig'], '--request-timeout=10s']
    cluster = json.loads(output(kube + ['get', 'nodes', '-o', 'json']))
    assert nodes == sorted(node['metadata']['name'] for node in cluster['items'])
    for node in cluster['items']:
        assert any(c['type'] == 'Ready' and c['status'] == 'True' for c in node['status']['conditions'])
    loaded = {}
    for node in nodes:
        images = json.loads(output(['docker', 'exec', node, 'crictl', 'images', '--output', 'json']))['images']
        selected = [entry for entry in images if 'docker.io/library/' + p['image'] in entry.get('repoTags', [])]
        assert len(selected) == 1 and selected[0]['id'] == image['Id'], node
        mapping = json.loads(output(['docker', 'exec', node, 'crictl', 'inspecti', image['Id']]))
        assert set(mapping['status']['repoDigests']) <= set(p['accepted_cri_image_ids'])
        assert mapping['status']['repoDigests']
        loaded[node] = selected[0]['id']
    ns = json.loads(output(kube + ['get', 'namespaces', '-o', 'json']))
    actual = {row['metadata']['name']: row['metadata']['uid'] for row in ns['items']}
    assert all(actual.get(name) == uid for name, uid in p['preserve_namespaces'].items())
    if not p['fixture_started']:
        assert all(name in p['preserve_namespaces'] or p.get('concurrent_owned_namespaces', {}).get(name) == uid for name, uid in actual.items()), actual
    if args.copy_workload or args.copy_native:
        assert p.get('plan_ready') and p['fixture_started'] and p['fixture_launch_authorized'], 'copy only inside the authorized prepared fixture'
    if args.copy_workload:
        assert args.copy_workload.parent.name.startswith('kv9-chaos-e2e.') and args.copy_workload.parent.parent.resolve() == Path(p['evidence_parent']).resolve()
        shutil.copytree(build, args.copy_workload)
        for source in build.iterdir():
            if source.is_file():
                assert sha(source) == sha(args.copy_workload / source.name)
    if args.copy_native:
        assert args.copy_native.parent.name.startswith('kv9-chaos-e2e.') and args.copy_native.parent.parent.resolve() == Path(p['evidence_parent']).resolve()
        shutil.copytree(native, args.copy_native)
        for source in native.iterdir():
            if source.is_file():
                assert sha(source) == sha(args.copy_native / source.name)
    print(json.dumps(dict(complete=True, revision=p['revision'], image_id=image['Id'], nodes=loaded,
                         source_files=len(p['source']), client_sha256=client['binary_sha256'], native_client_sha256=native_client['binary_sha256'],
                         preserved_namespaces=len(p['preserve_namespaces']), copied_workload=str(args.copy_workload) if args.copy_workload else None)))


if __name__ == '__main__':
    main()
