"""Check prepared source/image/readiness and freeze inputs; never launch faults."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--plan', type=Path, required=True)
    args = ap.parse_args()
    plan_path = args.plan.resolve()
    p = json.loads(plan_path.read_text())
    assert p['build_complete'] and not p['fixture_started'] and not p['fixture_launch_authorized']
    assert not p.get('plan_ready'), 'do not refreeze an existing plan'
    output = plan_path.parent/'readiness'
    output.mkdir(exist_ok=False)
    commands = []

    def run(name, command):
        row = dict(command=command, started_unix_ns=time.time_ns(), exit_code=None)
        commands.append(row)
        try:
            with (output/(name+'.stdout')).open('x') as out, (output/(name+'.stderr')).open('x') as err:
                result = subprocess.run(command, stdout=out, stderr=err, timeout=60)
            row['exit_code'] = result.returncode
        except BaseException as error:
            row['failure'] = repr(error)
            raise
        finally:
            row['ended_unix_ns'] = time.time_ns()
            (output/'commands.json').write_text(json.dumps(commands, indent=2)+'\n')
        assert row['exit_code'] == 0, row
        return (output/(name+'.stdout')).read_text()

    k = ['kubectl', '--kubeconfig', p['kubeconfig'], '--request-timeout=10s']
    names = json.loads(run('namespaces', k+['get', 'namespaces', '-o', 'json']))
    assert {x['metadata']['name']: x['metadata']['uid'] for x in names['items']} == p['preserve_namespaces']
    pods = json.loads(run('chaos-pods', k+['get', 'pods', '-n', 'chaos-mesh', '-o', 'json']))
    assert pods['items'] and all(x['status']['phase']=='Running' and all(c['ready'] for c in x['status']['containerStatuses']) for x in pods['items'])
    run('crds', k+['get', 'crd', 'podchaos.chaos-mesh.org', 'networkchaos.chaos-mesh.org', 'iochaos.chaos-mesh.org', '-o', 'json'])
    # Historical faults are recorded, never deleted or adopted by this fixture.
    run('faults', k+['get', 'podchaos,networkchaos,iochaos', '-A', '-o', 'json'])
    run('disk', ['df', '-h', str(plan_path.parent)])
    run('verify', ['python3', str(Path(__file__).parent/'verify-prebuilt.py'), '--plan', str(plan_path)])
    p['readiness'] = dict(checked_unix_ns=time.time_ns(), commands=commands)
    p['plan_ready'] = True
    p['ready_plan_path'] = str(plan_path.parent/'ready-plan.json')
    with Path(p['ready_plan_path']).open('x') as out:
        out.write(json.dumps(p, indent=2)+'\n')
    p['ready_plan_sha256'] = sha(p['ready_plan_path'])
    plan_path.write_text(json.dumps(p, indent=2)+'\n')
    print(json.dumps(dict(ready=True, fixture_authorized=False, plan=str(plan_path), ready_plan_sha256=p['ready_plan_sha256'])))


if __name__ == '__main__':
    main()
