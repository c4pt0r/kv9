#!/usr/bin/env python3
"""Retain a release observer microexperiment and its exact revision/environment."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def run(*command):
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    metadata = {
        'revision': run('git', 'rev-parse', 'HEAD'), 'git_status': run('git', 'status', '--porcelain'),
        'profile': 'release', 'rustc': run('rustc', '-Vv'), 'cargo': run('cargo', '-V'),
        'kernel': platform.platform(), 'cpu': run('lscpu'),
        'affinity': sorted(os.sched_getaffinity(0)),
        'rustflags': os.environ.get('RUSTFLAGS', ''),
        'command': 'cargo run --locked --release -p kv9-server --example latency-overhead',
        'scope': 'observer recording and in-memory snapshot/JSON only; no filesystem publication, RPC throughput, or client latency claim',
        'sources': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in [
            'crates/common/src/metrics.rs', 'crates/server/examples/latency-overhead.rs',
            'crates/server/src/observability.rs', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']},
    }
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    with (output / 'raw.jsonl').open('w') as stdout, (output / 'build.log').open('w') as stderr:
        subprocess.run(['cargo', 'run', '--locked', '--release', '-p', 'kv9-server', '--example', 'latency-overhead'],
                       cwd=ROOT, stdout=stdout, stderr=stderr, check=True)
    rows = [json.loads(line) for line in (output / 'raw.jsonl').read_text().splitlines()]
    assert len(rows) == 42
    groups = {}
    for row in rows:
        if row['experiment'] == 'record':
            assert row['observed_samples'] == (row['operations'] if row['enabled'] else 0)
            name = f"record/threads={row['threads']}/enabled={row['enabled']}"
        else:
            assert row['metric_count'] == 26 and row['json_bytes'] < 512 * 1024
            name = f"snapshot_json/dense={row['dense']}"
        groups.setdefault(name, []).append(row['wall_ns_per_operation'])
    summary = {name: {'trials': len(values), 'min_wall_ns_per_operation': min(values),
                      'median_wall_ns_per_operation': statistics.median(values),
                      'max_wall_ns_per_operation': max(values)} for name, values in groups.items()}
    assert len(summary) == 6 and all(v['trials'] == 7 for v in summary.values())
    (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    print('PASS: 42 release observer trials retained with exact revision and environment; no server throughput claim')


if __name__ == '__main__':
    main()
