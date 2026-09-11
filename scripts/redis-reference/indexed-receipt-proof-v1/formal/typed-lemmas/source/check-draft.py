"""Check an isolated proof draft; this is not the final controlled acceptance gate."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    assert not out.exists() and not out.is_relative_to(HERE)
    assert set(os.sched_getaffinity(0)) <= set(range(6, 16)) | set(range(22, 32))
    out.mkdir()
    source = out / 'source'
    source.mkdir()
    copied = []
    for name in ['AppliedReceipts.tla', 'AppliedReceiptsProof.tla',
                 'AppliedReceiptsMC.cfg', 'inventory-draft.json', 'check-draft.py']:
        shutil.copyfile(HERE / name, source / name)
        copied.append((HERE / name, source / name))
    (source / 'helpers').mkdir()
    for name in ['check-tlaps.py', 'check-tla.py', 'ProofAudit.java']:
        shutil.copyfile(HERE / 'helpers' / name, source / 'helpers' / name)
        copied.append((HERE / 'helpers' / name, source / 'helpers' / name))
    bindings = {str(original): sha(copy) for original, copy in copied}
    assert all(sha(original) == digest for original, digest in
               ((original, bindings[str(original)]) for original, _ in copied))
    save(out / 'inputs.json', bindings)
    inventory = json.loads((source / 'inventory-draft.json').read_text())
    jar = Path('/tmp/kv9-p0-tools/tla2tools-v1.7.4.jar')
    tlapm = Path('/tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm').resolve()
    assert sha(jar) == inventory['sany_jar_sha256']
    assert subprocess.check_output([str(tlapm), '--version'], text=True).strip() == inventory['tlapm_version']
    classes = out / 'auditor'
    classes.mkdir()
    save(out / 'tools.json', {'jar': str(jar), 'jar_sha256': sha(jar),
                             'tlapm': str(tlapm), 'tlapm_sha256': sha(tlapm),
                             'tlapm_version': inventory['tlapm_version']})

    def run(name, argv, cwd, timeout):
        save(out / (name + '-invocation.json'),
             {'argv': argv, 'cwd': str(cwd), 'started_ns': time.time_ns()})
        with (out / (name + '.log')).open('x') as log:
            result = subprocess.run(argv, cwd=cwd, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=timeout)
        save(out / (name + '-terminal.json'),
             {'exit_code': result.returncode, 'terminal': True,
              'completed_ns': time.time_ns()})
        return result.returncode

    assert run('compile-auditor', ['javac', '-cp', str(jar), '-d', str(classes),
               str(source / 'helpers/ProofAudit.java')], source, 30) == 0
    spec = importlib.util.spec_from_file_location('proof', source / 'helpers/check-tlaps.py')
    proof = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(proof)

    class LocalJavaTemps:
        def __getattr__(self, name):
            return getattr(subprocess, name)

        def run(self, command, *positional, **keywords):
            if command[0] == 'java' and 'ProofAudit' in command:
                temporary = source / 'sany-temp'
                temporary.mkdir(exist_ok=True)
                command = [command[0], f'-Djava.io.tmpdir={temporary}', *command[1:]]
            return subprocess.run(command, *positional, **keywords)

    proof.subprocess = LocalJavaTemps()
    proof.audit(SimpleNamespace(jar=jar, classes=classes,
                                stdlib=tlapm.parent.parent / 'lib/tlapm/stdlib'),
                source, inventory)
    code = run('tlaps', [str(tlapm), '--strict', '--nofp', '--threads', '1',
               '--cache-dir', str(out / 'fresh-cache'),
               str(source / 'AppliedReceiptsProof.tla')], source, 300)
    output = (out / 'tlaps.log').read_text()
    counts = re.findall(r'\[INFO\]: All (\d+) obligations? proved\.', output)
    assert code == 0 and '[ERROR]' not in output and '[WARNING]' not in output
    assert len(counts) == 1 and int(counts[0]) > 0
    assert all(sha(Path(name)) == digest for name, digest in bindings.items())
    result = {'draft_proof_complete': True, 'formal_acceptance': False,
              'declared_theorems': len(inventory['modules']['AppliedReceiptsProof']['theorems']),
              'observed_fresh_obligations': int(counts[0]),
              'semantic_import_assumption_hole_audit': True,
              'original_sources_unchanged': True,
              'remaining': 'Freeze reviewed obligation count; complete finite configurations, attributable negative controls and final source mapping.'}
    save(out / 'result.json', result)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
