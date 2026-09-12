#!/usr/bin/env python3
"""Root owns the shared Cargo target for the bounded quorum trace source checks."""
import importlib.util
import json
import os
from pathlib import Path
import time

SOURCE = Path('/tmp/kv9-quorum-message-trace')
OUT = Path('/tmp/kv9-quorum-trace-source-first')
os.environ['CARGO_TARGET_DIR'] = '/home/dongxu/kv9/target'
os.environ['CARGO_BUILD_JOBS'] = '4'
os.environ['CARGO_NET_OFFLINE'] = 'true'
os.environ['CARGO_TERM_VERBOSE'] = 'true'


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    OUT.mkdir()
    spec = importlib.util.spec_from_file_location('window_builder', SOURCE / 'scripts/build-workload.py')
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    before = builder.snapshot()
    save(OUT / 'sources-before.json', before)
    prepared = json.loads(Path('/tmp/kv9-quorum-trace-source-root-first/commands-source.json').read_text())
    commands = [(row['name'], row['argv'], row['check_cargo_artifacts']) for row in prepared['source_gate']]
    result = dict(complete=False, source_root=str(SOURCE), commands=[])
    save(OUT / 'result.json', result)
    try:
        with builder.cache.BuildCache(SOURCE, OUT, True, before) as cache:
            for name, command, artifact_check in commands:
                row = dict(name=name, argv=command, started_ns=time.time_ns())
                result['commands'].append(row)
                save(OUT / 'result.json', result)
                print(name, 'started', flush=True)
                with (OUT / (name + '.stdout')).open('x') as stdout, (OUT / (name + '.stderr')).open('x') as stderr:
                    code = cache.run(command, stdout=stdout, stderr=stderr, timeout=1200).returncode
                row.update(exit_code=code, ended_ns=time.time_ns())
                if artifact_check:
                    cache.check_artifacts(OUT / (name + '.stdout'))
                if builder.snapshot() != before:
                    raise RuntimeError('Quorum trace source changed during checks')
                save(OUT / 'result.json', result)
                print(name, 'passed', flush=True)
        result['complete'] = True
        save(OUT / 'sources-after.json', builder.snapshot())
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        save(OUT / 'result.json', result)


if __name__ == '__main__':
    main()
