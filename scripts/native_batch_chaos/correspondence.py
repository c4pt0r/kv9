"""Compare maintained predicates with explicitly supplied executed and corrected source."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import ast
import difflib
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
TREE = HERE.parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def declarations(path):
    return {x.name: ast.dump(x, include_attributes=False) for x in ast.parse(path.read_text()).body
            if isinstance(x, (ast.FunctionDef, ast.ClassDef))}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--executed-overlay', type=Path, required=True)
    ap.add_argument('--corrected-check', type=Path, required=True)
    ap.add_argument('--base-revision', required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    pairs = []
    result = dict(accepted=False, comparisons=pairs)
    try:
        for name in ('native_protocol.py','native_capture.py','native-gate.py','native-final-drain.py','process-probe.py','observer-fields.py','delay-gate.py','observer.py'):
            before, after = a.executed_overlay/name, HERE/name
            old, new = declarations(before), declarations(after)
            assert old == new, ('predicate declaration changed', name, set(old)^set(new))
            pairs.append(dict(name=name, original_sha256=sha(before), maintained_sha256=sha(after), equal_declarations=sorted(old), byte_identical=before.read_bytes()==after.read_bytes()))
        old, new = declarations(a.corrected_check), declarations(HERE/'check-native-windows.py')
        assert set(old)==set(new)
        assert all(old[name]==new[name] for name in old if name!='main'), 'corrected checker predicate changed'
        pairs.append(dict(name='check-native-windows.py', original_sha256=sha(a.corrected_check), maintained_sha256=sha(HERE/'check-native-windows.py'), equal_declarations=sorted(set(old)-{'main'}), main_change='optional output path only'))
        assert (TREE/'chaos/native-batch.Dockerfile').read_bytes()==(a.executed_overlay/'Dockerfile').read_bytes()
        base = subprocess.check_output(['git','show',a.base_revision+':scripts/chaos-mesh-e2e.sh'],cwd=TREE,text=True)
        current = (TREE/'scripts/chaos-mesh-e2e.sh').read_text()
        removed=[]
        for tag,i,j,k,l in difflib.SequenceMatcher(None,base.splitlines(),current.splitlines()).get_opcodes():
            if tag in ('delete','replace'):
                removed.extend(base.splitlines()[i:j])
        assert removed==['artifact="$(mktemp -d /tmp/kv9-chaos-e2e.XXXXXX)"'], ('original command/predicate removed', removed)
        persistent=subprocess.check_output(['git','show',a.base_revision+':scripts/chaos-mesh-persistent.sh'],cwd=TREE)
        assert persistent==(TREE/'scripts/chaos-mesh-persistent.sh').read_bytes()
        for label, before in [('base',base),('executed',(a.executed_overlay/'scripts/chaos-mesh-e2e.sh').read_text())]:
            (a.output/(label+'-fixture.diff')).write_text(''.join(difflib.unified_diff(before.splitlines(True), current.splitlines(True), fromfile=label, tofile='maintained')))
        result.update(accepted=True, original_point_persistent_byte_identical=True, original_driver_predicates_preserved=True,
                      original_image_payload_recipe_byte_identical=True, scope='Source correspondence only. No new fixture or runtime acceptance.')
    except BaseException as error:
        result['failure']=repr(error)
        raise
    finally:
        (a.output/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='comparisons'}))


if __name__=='__main__':
    main()
