#!/usr/bin/env python3
"""Run a separately labeled volatile tmpfs diagnostic; never durable acceptance."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from benchmark import Fixture, save

SPEC = importlib.util.spec_from_file_location('reference', Path(__file__).with_name('redis-comparison.py'))
reference = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reference)
VOLATILE = 'Volatile tmpfs data; normal Raft quorum and sync calls, but NO disk durability or power-loss guarantee'


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def files(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('tmpfs retention refuses symbolic links')
        if path.is_dir():
            continue
        if not path.is_file():
            raise ValueError('tmpfs retention requires regular files')
        result[str(path.relative_to(root))] = dict(bytes=path.stat().st_size, sha256=digest(path))
    return result


class TmpfsFixture(Fixture):
    def __init__(self, out, target, build, placement):
        if target != 'wal' or shutil.disk_usage('/dev/shm').free < 32 * 1024**3:
            raise ValueError('diagnostic requires WAL mode and at least 32 GiB free tmpfs')
        super().__init__(out, target, build, placement)
        self.scratch = Path(tempfile.mkdtemp(prefix='kv9-ready-tmpfs-', dir='/dev/shm'))
        self.link = self.out / 'data'
        self.link.symlink_to(self.scratch, target_is_directory=True)
        self.record.update(diagnostic_only=True, power_loss_durability=False, actual_storage=VOLATILE)
        save(self.out / 'tmpfs-created.json', dict(diagnostic_only=True, volatile=True,
             power_loss_durability=False, scratch=str(self.scratch), logical_data=str(self.link)))

    def start(self):
        super().start()
        observed = []
        for node, process in self.nodes.items():
            logical = self.link / f'n{node}'
            physical = logical.resolve()
            if not physical.is_relative_to(self.scratch):
                raise ValueError('voter data escaped owned tmpfs')
            command = ['findmnt', '--json', '--target', str(physical), '--output', 'TARGET,FSTYPE,SOURCE,OPTIONS']
            result = subprocess.run(command, text=True, capture_output=True, check=True, timeout=10)
            mount = json.loads(result.stdout)
            if len(mount['filesystems']) != 1 or mount['filesystems'][0]['fstype'] != 'tmpfs':
                raise ValueError('actual voter data is not tmpfs')
            stat = physical.stat()
            args = Path(f'/proc/{process.pid}/cmdline').read_bytes().decode().rstrip('\0').split('\0')
            if args[args.index('--data-dir') + 1] != str(logical):
                raise ValueError('running voter uses an unexpected data directory')
            observed.append(dict(node=node, pid=process.pid, logical_data=str(logical),
                physical_data=str(physical), device=f'{os.major(stat.st_dev)}:{os.minor(stat.st_dev)}',
                command_line=args, mount_command=command, mount=mount,
                mountinfo=Path(f'/proc/{process.pid}/mountinfo').read_text(),
                executable_sha256=self.identities[process.pid]['executable_sha256']))
        save(self.out / 'tmpfs-voters.json', dict(diagnostic_only=True, volatile=True,
             power_loss_durability=False, voters=observed))

    def snapshot(self, directory, phase):
        if shutil.disk_usage(self.scratch).free < 8 * 1024**3:
            raise ValueError('tmpfs free-space floor reached; partial artifacts retained')
        super().snapshot(directory, phase)

    def close(self):
        # On any failure, stop children before touching their data. If retention
        # fails, the owned tmpfs directory remains available for diagnosis.
        super().close()
        if any(p.poll() is None for p in self.children):
            raise ValueError('cannot retain data while an owned child is alive')
        before = files(self.scratch)
        copied = self.out / 'retained-tmpfs-data'
        shutil.copytree(self.scratch, copied)
        if files(copied) != before or files(self.scratch) != before:
            raise ValueError('tmpfs retention hashes differ; original scratch preserved')
        if not self.link.is_symlink() or self.link.resolve() != self.scratch:
            raise ValueError('refusing to replace an unexpected data link')
        self.link.unlink()
        copied.rename(self.link)
        if self.scratch.parent != Path('/dev/shm') or not self.scratch.name.startswith('kv9-ready-tmpfs-'):
            raise ValueError('refusing to clean unowned scratch')
        shutil.rmtree(self.scratch)
        save(self.out / 'tmpfs-retention.json', dict(complete=True, children_exited=True,
             original_scratch=str(self.scratch), scratch_removed=not self.scratch.exists(),
             retained_data=str(self.link), original_runtime_storage='volatile tmpfs',
             retained_copy_storage='post-run disk evidence copy; not runtime durability', files=before))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--build', type=Path, required=True)
    parser.add_argument('--redis-client', type=Path, required=True)
    parser.add_argument('--expected-revision', required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    original_argv = list(sys.argv)
    wrapper_sha = digest(__file__)
    original_save = reference.save

    def labeled_save(path, value):
        if Path(path).name == 'protocol.json':
            value.update(diagnostic_only=True, volatile=True, power_loss_durability=False,
                         actual_storage='tmpfs', kv9_durability=VOLATILE,
                         diagnostic_wrapper_sha256=wrapper_sha)
            value['limitations'].append('Volatile tmpfs diagnostic; never combine with durable disk acceptance or throughput claims')
            shutil.copy2(__file__, output / 'diagnostic-wrapper.py')
            original_save(output / 'diagnostic-invocation.json', dict(argv=original_argv,
                          wrapper_sha256=wrapper_sha, diagnostic_only=True))
        original_save(path, value)

    reference.save = labeled_save
    reference.Fixture = TmpfsFixture
    sys.argv = [__file__, '--output', str(output), '--build', str(args.build),
                '--redis-client', str(args.redis_client), '--expected-revision', args.expected_revision,
                '--concurrency', '1,64', '--repetitions', '2', '--measure-ms', '3000', '--keys', '64']
    print('VOLATILE TMPFS DIAGNOSTIC: no disk durability or power-loss guarantee', flush=True)
    try:
        reference.main()
    except BaseException as error:
        if output.exists():
            save(output / 'diagnostic-exit.json', dict(exit_code=1, error=repr(error), diagnostic_only=True))
        raise
    else:
        save(output / 'diagnostic-exit.json', dict(exit_code=0, diagnostic_only=True))


if __name__ == '__main__':
    main()
