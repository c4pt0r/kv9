#!/usr/bin/env python3
"""Show that independent model/deep-stack tests reject deliberate index faults."""
import hashlib
import json
from pathlib import Path
import resource
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
source = (REPO / 'scripts/resident-radix/src/lib.rs').read_text()
tests = (REPO / 'scripts/resident-radix/src/tests.rs').read_bytes()
digest = lambda data: hashlib.sha256(data).hexdigest()
drop_start = source.index('impl Drop for Node {')
drop_end = source.index('\nfn common_prefix(', drop_start)
controls = [
    ('discard-unique-leaf-overwrite', '*old = entry;', 'drop(entry);',
     'all_short_prefix_transitions_with_old_roots', 'model'),
    ('keep-consumed-prefix-edge', 'old.prefix.drain(..=common);', 'old.prefix.drain(..common);',
     'all_short_prefix_transitions_with_old_roots', 'model'),
    ('omit-collapse-edge', 'prefix.push(edge.byte);', 'let _ = edge.byte;',
     'all_short_prefix_transitions_with_old_roots', 'model'),
    ('skip-inclusive-terminal', 'if inclusive {', 'if false {',
     'all_bound_kinds_and_mixed_iteration_match_btree', 'model'),
    ('reverse-cursor-direction', 'if self.reverse {', 'if !self.reverse {',
     'all_bound_kinds_and_mixed_iteration_match_btree', 'model'),
    ('recursive-node-teardown', source[drop_start:drop_end],
     'impl Drop for Node { fn drop(&mut self) {} }\n',
     'deep_prefix_updates_iteration_and_reclamation_on_small_stack', 'stack-overflow'),
]
result = {'complete': False, 'started_ns': time.time_ns(), 'controls': [],
          'source_sha256': digest(source.encode()), 'tests_sha256': digest(tests),
          'scope': 'Compiled Rust mutation tests, not a formal proof.'}
try:
    for name, old, new, test, kind in controls:
        assert source.count(old) == 1, name
        work = OUT / name
        work.mkdir()
        mutated = source.replace(old, new)
        (work / 'lib.rs').write_text(mutated)
        (work / 'tests.rs').write_bytes(tests)
        with (work / 'build.stdout').open('x') as stdout, (work / 'build.stderr').open('x') as stderr:
            built = subprocess.run(['rustc', '--edition=2021', '--test', 'lib.rs', '-o', 'control'],
                                   cwd=work, stdout=stdout, stderr=stderr, timeout=120)
        assert built.returncode == 0, f'{name}: must compile before test rejection'
        def no_core():
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        with (work / 'test.stdout').open('x') as stdout, (work / 'test.stderr').open('x') as stderr:
            tested = subprocess.run([str(work / 'control'), '--exact', 'tests::' + test,
                                     '--nocapture', '--test-threads=1'],
                                    cwd=work, stdout=stdout, stderr=stderr, timeout=120,
                                    preexec_fn=no_core)
        stdout = (work / 'test.stdout').read_text()
        stderr = (work / 'test.stderr').read_text()
        if kind == 'model':
            assert tested.returncode == 101 and '1 failed' in stdout and 'assertion' in stderr, name
        else:
            assert tested.returncode == -6 and 'stack overflow' in stderr, (name, tested.returncode)
        result['controls'].append({'name': name, 'compiled': True, 'rejected': True,
            'exit_code': tested.returncode, 'test': test, 'failure_kind': kind,
            'mutant_sha256': digest(mutated.encode()), 'binary_sha256': digest((work / 'control').read_bytes())})
        print(json.dumps(result['controls'][-1]), flush=True)
    assert (REPO / 'scripts/resident-radix/src/lib.rs').read_text() == source
    assert (REPO / 'scripts/resident-radix/src/tests.rs').read_bytes() == tests
    result['complete'] = True
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
