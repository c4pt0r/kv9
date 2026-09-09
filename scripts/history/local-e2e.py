#!/usr/bin/env python3
"""Three-process acceptance for the independent history recorder/checker."""
import argparse
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from root_provision import prepare_stores


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-port', type=int, default=28300)
    parser.add_argument('--duration', type=int, default=15)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    binary = repo / 'target/debug/kv9'
    artifact = Path(tempfile.mkdtemp(prefix='kv9-history-local-'))
    artifact.chmod(0o700)
    print(f'Artifacts: {artifact}', flush=True)
    token = secrets.token_hex(16)
    env = {**os.environ, 'KV9_BOOTSTRAP_TOKEN': token, 'KV9_CLUSTER_TOKEN': token,
           'KV9_CLIENT_TOKENS': 'history='+token, 'KV9_CLIENT_TOKEN': token}
    endpoints = [f'127.0.0.1:{args.base_port+i}' for i in [1, 2, 3]]
    processes, logs = [], []
    def run(argv, **kw):
        return subprocess.run(list(map(str, argv)), env=env, check=True, **kw)
    try:
        prepared = prepare_stores(lambda argv: run(argv, capture_output=True, text=True).stdout,
                                  binary, {i: artifact/f'n{i}' for i in (1, 2, 3)})
        run([binary, 'root-create', '--output', artifact/'root.bin', '--voters', ','.join(f'{i}@{addr}' for i, addr in enumerate(endpoints, 1)), '--store-incarnations', prepared], stdout=subprocess.DEVNULL)
        for i, addr in enumerate(endpoints, 1):
            data = artifact/f'n{i}'
            run([binary, 'init', '--root', artifact/'root.bin', '--node-id', i, '--data-dir', data], stdout=subprocess.DEVNULL)
            log = (artifact/f'n{i}.log').open('w'); logs.append(log)
            processes.append(subprocess.Popen([str(binary), 'start', '--node-id', str(i), '--addr', addr, '--data-dir', str(data)], env=env, stdout=log, stderr=log))
        run([sys.executable, repo/'scripts/history/workload.py', '--binary', binary, '--addresses', ','.join(endpoints),
             '--name', 'history-local', '--history', artifact/'history.jsonl', '--duration', args.duration], timeout=args.duration+80)
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError('a database process died during history acceptance')
        run([sys.executable, repo/'scripts/history/checker.py', artifact/'history.jsonl', '--output', artifact/'checker.json',
             '--acceptance', '--require', 'put', 'get', 'delete', 'scan', 'delete_range', 'create_keyspace', '--seconds', '30'], timeout=45)
        print('PASS: three-process Raw KV/catalog history accepted with all six APIs', flush=True)
    finally:
        for process in processes:
            if process.poll() is None: process.terminate()
        for process in processes:
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
        for log in logs: log.close()


if __name__ == '__main__':
    main()
