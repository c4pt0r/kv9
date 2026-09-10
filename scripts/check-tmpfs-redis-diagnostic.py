#!/usr/bin/env python3
"""Recheck complete trials plus volatile-storage identity and retained tmpfs bytes."""
import argparse
import copy
import hashlib
import importlib.util
import json
from pathlib import Path


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


standard = load_module('standard_check', 'check-redis-comparison.py')
wrapper = load_module('tmpfs_wrapper', 'tmpfs-redis-diagnostic.py')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def audit(root, build, overrides=None):
    overrides = overrides or {}
    def read(relative):
        return overrides.get(relative, json.loads((root / relative).read_text()))
    protocol = read('protocol.json')
    require(protocol.get('diagnostic_only') is True and protocol.get('volatile') is True
            and protocol.get('power_loss_durability') is False and protocol.get('actual_storage') == 'tmpfs'
            and protocol.get('kv9_durability') == wrapper.VOLATILE,
            'volatile/no-power-loss diagnostic labels are required')
    require(protocol['concurrency'] == [1, 64] and protocol['repetitions'] == 2
            and protocol['measure_ms'] == 3000 and protocol['keys'] == 64
            and protocol['value_bytes'] == 128 and protocol['warmup_operations'] == 32,
            'diagnostic protocol must contain the bounded 24 trials')
    require(wrapper.digest(root / 'diagnostic-wrapper.py') == protocol['diagnostic_wrapper_sha256'],
            'retained wrapper differs from execution identity')
    require(read('diagnostic-exit.json')['exit_code'] == 0, 'diagnostic did not exit successfully')
    created = read('kv9/tmpfs-created.json')
    scratch = Path(created['scratch'])
    require(scratch.parent == Path('/dev/shm') and scratch.name.startswith('kv9-ready-tmpfs-'),
            'scratch is not an owned tmpfs path')
    voters = read('kv9/tmpfs-voters.json')['voters']
    require({v['node'] for v in voters} == {1, 2, 3} and len(voters) == 3, 'all three voter observations required')
    expected_binary = json.loads((build / 'build.json').read_text())['binaries']['kv9']['sha256']
    fixture = read('kv9/fixture.json')
    require(fixture['diagnostic_only'] is True and fixture['power_loss_durability'] is False,
            'fixture durability label differs')
    for voter in voters:
        node = voter['node']
        require(Path(voter['physical_data']) == scratch / f'n{node}', 'voter data is outside owned tmpfs')
        mounts = voter['mount']['filesystems']
        require(len(mounts) == 1 and mounts[0]['fstype'] == 'tmpfs', 'voter mount is not tmpfs')
        require(any(line.split()[2] == voter['device'] and ' - tmpfs ' in line
                    for line in voter['mountinfo'].splitlines()), 'voter process lacks the observed tmpfs device')
        argv = voter['command_line']
        require(argv[argv.index('--data-dir') + 1] == voter['logical_data'], 'voter command used a different data directory')
        require(voter['executable_sha256'] == expected_binary
                and fixture['process_ids'][str(node)] == voter['pid']
                and fixture['process_identities'][str(node)]['executable_sha256'] == expected_binary,
                'actual voter identity differs from exact release build')
    retained = read('kv9/tmpfs-retention.json')
    require(retained['complete'] and retained['children_exited'] and retained['scratch_removed']
            and not scratch.exists(), 'owned scratch was not retained and cleaned after child exit')
    require(retained['original_scratch'] == str(scratch)
            and retained['original_runtime_storage'] == 'volatile tmpfs', 'retained origin differs')
    require(wrapper.files(root / 'kv9/data') == retained['files'], 'retained tmpfs data differs from copy-time hashes')
    matrix = read('matrix.json')
    require(matrix['placement']['client_cpus'] == [0, 1]
            and matrix['placement']['server_cpus'] == [2, 3, 4, 5], 'diagnostic CPU placement differs')
    for trial in matrix['attempts']:
        if trial['target'] != 'kv9':
            continue
        config = read(f"kv9/{trial['name']}/requested-config.json")
        require(config['client']['deadline_ms'] == 1500 and config['client']['max_attempts'] == 6
                and config['client']['max_in_flight'] == trial['workers'], 'diagnostic client limits changed')
    return dict(diagnostic_only=True, volatile=True, power_loss_durability=False,
                retained_files=len(retained['files']), retained_bytes=sum(x['bytes'] for x in retained['files'].values()),
                original_scratch=str(scratch), scratch_removed=True)


def controls(root, build):
    protocol = json.loads((root / 'protocol.json').read_text())
    mounts = json.loads((root / 'kv9/tmpfs-voters.json').read_text())
    retained = json.loads((root / 'kv9/tmpfs-retention.json').read_text())
    a = copy.deepcopy(protocol); a['power_loss_durability'] = True
    b = copy.deepcopy(mounts); b['voters'][0]['mount']['filesystems'][0]['fstype'] = 'ext4'
    c = copy.deepcopy(mounts); c['voters'][0]['physical_data'] = '/tmp/foreign-data'
    d = copy.deepcopy(retained); d['files'][next(iter(d['files']))]['sha256'] = '0' * 64
    cases = [('false-durability', 'protocol.json', a, 'volatile/no-power-loss diagnostic labels are required'),
             ('disk-mount', 'kv9/tmpfs-voters.json', b, 'voter mount is not tmpfs'),
             ('foreign-data', 'kv9/tmpfs-voters.json', c, 'voter data is outside owned tmpfs'),
             ('corrupt-copy', 'kv9/tmpfs-retention.json', d, 'retained tmpfs data differs from copy-time hashes')]
    results = []
    for name, relative, mutant, expected in cases:
        try:
            audit(root, build, {relative: mutant})
        except ValueError as error:
            require(str(error) == expected, f'{name}: failed outside its intended assertion')
        else:
            raise ValueError(f'{name}: invalid diagnostic evidence accepted')
        results.append(dict(name=name, expected_rejection=expected,
                            mutant_sha256=hashlib.sha256(json.dumps(mutant, sort_keys=True).encode()).hexdigest()))
    audit(root, build)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('matrix', 'build', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--expected-revision', required=True)
    args = parser.parse_args()
    with args.output.open('x') as stream:
        stream.write('{"complete": false}\n')
    extra = audit(args.matrix, args.build)
    result = standard.check(args.matrix, args.build, args.expected_revision)
    require(result['trials'] == 24, 'not all 24 diagnostic trials were checked')
    result.update(storage_audit=extra, evidence_controls=controls(args.matrix, args.build))
    args.output.write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    print('PASS: 24 volatile tmpfs diagnostic trials, exact storage identity/retention, four invalid-evidence controls')


if __name__ == '__main__':
    main()
