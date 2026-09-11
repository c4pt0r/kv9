"""Read-only exact release binding; never build, load images or contact Kubernetes."""
if not __debug__:
    raise RuntimeError("source verification requires PYTHONOPTIMIZE=0")

import hashlib
import json
from pathlib import Path
import subprocess


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def load(path):
    return json.loads(Path(path).read_text())


def verify(plan, require_image=True):
    source_plan = Path(plan['source_plan'])
    assert sha(source_plan) == plan['source_plan_sha256'], 'source binding changed'
    base = load(source_plan)
    tree = Path(plan['worktree'])
    revision = '9be0c1963515ff974426faadef80482b847b13a5'
    assert base['revision'] == plan['revision'] == revision
    assert base['dirty'] is False and base['profile'] == 'release'
    assert subprocess.check_output(['git', '-C', str(tree), 'rev-parse', 'HEAD'], text=True).strip() == revision
    assert not subprocess.check_output(['git', '-C', str(tree), 'status', '--porcelain', '--untracked-files=all'])
    names = set(subprocess.check_output(['git', '-C', str(tree), 'ls-files', '-z'], text=True).strip('\0').split('\0'))
    assert names == set(base['source']), 'source inventory incomplete'
    for name, expected in base['source'].items():
        assert sha(tree / name) == expected, name
    for name, expected in base['files'].items():
        assert sha(name) == expected, name
    digest = hashlib.sha256(json.dumps(base['source'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    assert digest == base['source_tree_sha256']
    build = Path(base['build_directory'])
    native = Path(plan['native_build_directory'])
    assert native == build / 'workload'
    server = load(build / 'build.json')
    manifest = load(native / 'build.json')
    sources = load(native / 'sources.json')
    assert manifest == plan['native_client_build']
    for record in (server, manifest, sources):
        assert record['revision'] == revision and record['dirty'] is False
        assert record['source_tree_sha256'] == digest
    assert server['profile'] == manifest['profile'] == 'release'
    assert server['rustc'] == manifest['rustc']
    assert sources['sources'] == server['sources'] == base['source']
    assert server['workload_build_sha256'] == sha(native / 'build.json')
    assert sha(native / 'kv9-batch-workload') == manifest['binary_sha256'] == sources['binary_sha256']
    assert Path(plan['production_binary']['path']) == build / 'kv9'
    assert sha(build / 'kv9') == plan['production_binary']['sha256'] == server['binaries']['kv9']['sha256']
    assert server['binaries']['kv9']['command'] == ['cargo', 'build', '--locked', '--bin', 'kv9', '--release', '--message-format=json-render-diagnostics']
    assert sources['command'] == ['cargo', 'build', '--locked', '-p', 'kv9-server', '--bin', 'kv9-batch-workload', '--message-format=json-render-diagnostics', '--release']
    features = {}
    for path, target in ((build / 'kv9-cargo.jsonl', 'kv9'), (native / 'cargo.jsonl', 'kv9-batch-workload')):
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        assert rows[-1] == {'reason': 'build-finished', 'success': True}
        found = {}
        for name in (target, 'kv9_engine', 'kv9_raft', 'kv9_server'):
            artifacts = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == name]
            assert len(artifacts) == 1, (path, name)
            artifact = artifacts[0]
            assert artifact['features'] == [] and artifact['profile']['test'] is False
            assert artifact['profile']['opt_level'] == '3'
            assert artifact['package_id'].startswith('path+file://' + str(tree)), 'different build source tree'
            found[name] = {key: artifact[key] for key in ('features', 'profile', 'package_id', 'executable')}
        features[str(path)] = {'sha256': sha(path), 'artifacts': found}
    for row in plan['source_default_limits']['sources']:
        assert sha(row['path']) == row['sha256']
    assert plan['source_default_limits']['public_rpc_limit_requests'] == 64
    assert plan['source_default_limits']['public_rpc_limit_encoded_bytes'] == 64 * 1024 * 1024
    assert plan['source_default_limits']['raft_async_read_limit'] == plan['source_default_limits']['raft_async_apply_limit'] == 128
    assert plan['pod_limit_overrides'] == {}
    image = None
    if require_image:
        assert plan['image_ready'] is True, 'image build/load/attestation is pending'
        assert sha(plan['image_binding']) == plan['image_binding_sha256'], 'image binding changed'
        image = load(plan['image_binding'])
        assert image['accepted'] is True and image['image'] == plan['image'] and image['revision'] == revision
        assert image['source_plan_sha256'] == sha(source_plan)
        assert image['accepted_cri_image_ids'] == plan['accepted_cri_image_ids'] and plan['accepted_cri_image_ids']
        assert image['payload_sha256'] == {'/usr/local/bin/kv9': plan['production_binary']['sha256'], '/usr/local/bin/kv9-batch-workload': manifest['binary_sha256'], '/opt/kv9-batch-workload/build.json': sha(native / 'build.json')}
        for path, expected in image['evidence_files'].items():
            assert sha(path) == expected, 'image evidence changed: ' + path
    return {'accepted': True, 'scope': 'Exact retained release source/build binding only; running PID and image identity remain independently checked by the fixture',
            'revision': revision, 'source_files': len(base['source']), 'source_plan_sha256': sha(source_plan),
            'source_tree_sha256': digest, 'server': plan['production_binary'], 'native_client_sha256': manifest['binary_sha256'],
            'cargo_features': features, 'native_manifest_sha256': sha(native / 'build.json'),
            'image_required': require_image, 'image_binding': image}


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--preparation-without-image', action='store_true')
    args = parser.parse_args()
    assert not args.output.exists()
    result = verify(load(args.plan), require_image=not args.preparation_without_image)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    print('PASS: exact source and standalone release default feature graphs; image_required=' + str(result['image_required']))
