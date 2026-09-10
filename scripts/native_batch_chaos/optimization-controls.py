"""Safe optimized-mode refusal controls; prohibit child mutations and process/network launches."""
if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")

import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
REFUSAL = 'FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.'
TRAP = 'CONTROL: forbidden side effect'
# This wrapper only reads/compiles the actual source. Its audit hook is active
# before run_path and prevents filesystem mutation, subprocesses and networking,
# including if a future regression removes or moves the optimized-mode guard.
WRAPPER = r'''
import json, os, runpy, sys
source, arguments = sys.argv[1], json.loads(sys.argv[2])
def forbid(event, args):
    mutation = event in {
        'os.mkdir', 'os.remove', 'os.rmdir', 'os.rename', 'os.link', 'os.symlink',
        'os.chmod', 'os.chown', 'os.truncate', 'os.utime', 'os.system',
        'os.posix_spawn', 'os.fork', 'os.forkpty', 'os.exec', 'subprocess.Popen',
        'socket.connect', 'socket.connect_ex', 'socket.bind', 'socket.sendto',
    }
    if event == 'open':
        _, mode, flags = args
        mutation = bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND))
        mutation = mutation or bool(mode and any(c in mode for c in 'wax+'))
    if mutation:
        raise RuntimeError('CONTROL: forbidden side effect: ' + event)
sys.addaudithook(forbid)
sys.argv = [source, *arguments]
sys.path.insert(0, os.path.dirname(source))
runpy.run_path(source, run_name='__main__')
'''


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inventory(path):
    return {str(p.relative_to(path)): digest(p) for p in sorted(path.rglob('*')) if p.is_file()}


def guard_statement(source):
    statements = ast.parse(source).body
    assert isinstance(statements[0], ast.Expr) and isinstance(statements[0].value, ast.Constant)
    guard = statements[1]
    expected = ast.parse('if not __debug__:\n    raise SystemExit('+repr(REFUSAL)+')').body[0]
    assert ast.dump(guard, include_attributes=False) == ast.dump(expected, include_attributes=False), 'guard must precede imports and side effects'
    return guard


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    assert not out.is_relative_to(HERE.parents[1]), 'controls must be outside source'
    out.mkdir(parents=True, exist_ok=False)
    rows = []
    result = dict(accepted=False, cases=rows, scope=__doc__, actual_builds=0, actual_launches=0, actual_network_operations=0)
    (out/'wrapper.py').write_text(WRAPPER)
    sources = sorted(HERE.glob('*.py'))
    fixture = out/'protected'
    fixture.mkdir()
    (fixture/'plan.json').write_text(json.dumps(dict(fixture_launch_authorized=True, plan_ready=True,
        build_complete=True, fixture_started=False, worktree=str(fixture), artifact=str(fixture/'raw')))+'\n')
    (fixture/'sentinel').write_text('Must remain unchanged.\n')
    before = inventory(fixture)
    default_args = ['--plan',str(fixture/'plan.json'),'--artifact',str(fixture/'raw')]
    commands = {
        'prepare.py': ['--output',str(fixture/'new-output'),'--target',str(fixture/'new-target'),
                       '--kubeconfig',str(fixture/'kubeconfig'),'--kind',str(fixture/'kind'),
                       '--kind-cluster','never-launch','--image','kv9-chaos:never-build'],
        'run.py': ['--plan',str(fixture/'plan.json')],
        'freeze.py': ['--plan',str(fixture/'plan.json')],
        'verify-prebuilt.py': ['--plan',str(fixture/'plan.json'),'--copy-workload',str(fixture/'new-output')],
        'observer.py': ['--plan',str(fixture/'plan.json')],
        'point-client-identity.py': [*default_args,'--namespace','kv9-chaos-never-launch'],
        'native-gate.py': [*default_args,'--namespace','kv9-chaos-never-launch','--phase','baseline'],
        'native-final-drain.py': [*default_args,'--namespace','kv9-chaos-never-launch'],
        'readback.py': [*default_args,'--output',str(fixture/'new-output')],
        'check-native-windows.py': [*default_args,'--output',str(fixture/'new-output')],
        'timestamp-controls.py': ['--output',str(fixture/'new-output'),'--timestamp',str(HERE/'timestamp-original.txt')],
        'window-controls.py': [*default_args,'--output',str(fixture/'new-output')],
        'config-drain-controls.py': ['--output',str(fixture/'new-output')],
        'optimization-controls.py': ['--output',str(fixture/'new-output')],
        'correspondence.py': ['--executed-overlay',str(fixture),'--corrected-check',str(fixture/'old.py'),
                              '--base-revision','HEAD','--output',str(fixture/'new-output')],
    }

    def run(name, source, argv, flags=(), optimize='0', expected=REFUSAL):
        command = [sys.executable, *flags, str(out/'wrapper.py'), str(source), json.dumps(argv)]
        row = dict(name=name, command=command, source_sha256=digest(source), python_optimize=optimize,
                   started_unix_ns=time.time_ns())
        rows.append(row)
        child = subprocess.run(command, cwd=fixture, env=dict(os.environ, PYTHONOPTIMIZE=optimize,
            PYTHONDONTWRITEBYTECODE='1'), text=True, capture_output=True, timeout=10)
        row.update(ended_unix_ns=time.time_ns(), exit_code=child.returncode,
                   stdout=name+'.stdout', stderr=name+'.stderr')
        (out/row['stdout']).write_text(child.stdout)
        (out/row['stderr']).write_text(child.stderr)
        assert inventory(fixture)==before and not (fixture/'new-output').exists() and not (fixture/'new-target').exists(), 'child mutated protected inputs/output'
        if expected is None:
            assert child.returncode==0 and TRAP not in child.stderr, (name, child.stderr)
        elif expected==REFUSAL:
            assert child.returncode==1 and child.stderr.strip()==REFUSAL and not child.stdout, (name,child.stderr)
            assert TRAP not in child.stderr, 'guard did not reject before side effects'
        else:
            assert child.returncode!=0 and expected in child.stderr, (name,child.stderr)
        row['accepted']=True

    try:
        for source in sources:
            guard_statement(source.read_text())
            argv=commands.get(source.name,default_args)
            for mode, flags, optimize in [('environment-1',(), '1'),('cli-O',('-O',), '0'),('cli-OO',('-OO',), '0')]:
                run(source.stem+'-'+mode,source,argv,flags,optimize)
            # Normal --help proves entrypoints still parse/import without any
            # protected mutation. Library-only modules simply finish normally.
            run(source.stem+'-normal-help',source,['--help'],expected=None)
        mutants=out/'guardless-controls';mutants.mkdir()
        for name in ('prepare.py','run.py','readback.py'):
            source=HERE/name; text=source.read_text(); guard=guard_statement(text)
            lines=text.splitlines(keepends=True)
            mutant=mutants/name
            mutant.write_text(''.join(lines[:guard.lineno-1]+lines[guard.end_lineno:]))
            run(name+'-guard-removed',mutant,commands[name],('-O',),'0',TRAP)
        assert inventory(fixture)==before
        result.update(accepted=True, guarded_files=len(sources), optimized_refusals=3*len(sources),
                      normal_help_checks=len(sources), guard_removal_traps=3,
                      source_hashes={str(p):digest(p) for p in sources}, protected_inventory=before)
    except BaseException as error:
        result['failure']=repr(error)
        raise
    finally:
        (out/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ('accepted','guarded_files','optimized_refusals','normal_help_checks','guard_removal_traps')}))


if __name__=='__main__':
    main()
