"""Serialize retained builds and invalidate only first-party package artifacts."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


class BuildCache:
    """One clean/build/copy/readback transaction, shared by all component builds.

    The lock coordinates these helpers, not arbitrary raw Cargo invocations.
    Output must already be a newly created directory outside the source tree.
    """

    def __init__(self, root, output, release, source, manifest=None):
        self.root = Path(root).resolve()
        self.output = Path(output).resolve()
        self.manifest = Path(manifest or self.root / 'Cargo.toml').resolve()
        self.release = release
        self.lock = None
        self.compiled = set()
        self.record = {
            'version': 1, 'complete': False, 'pid': os.getpid(),
            'source_root': str(self.root), 'manifest': str(self.manifest),
            'source_identity_sha256': hashlib.sha256(json.dumps(
                source, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
            'profile': 'release' if release else 'debug',
            'started_ns': time.time_ns(), 'commands': [], 'first_party_artifacts': [],
            'scope': 'Cooperating retained-build helpers; raw Cargo still requires external coordination',
        }

    def save(self):
        (self.output / 'cache-safety.json').write_text(
            json.dumps(self.record, sort_keys=True, indent=2) + '\n')

    def __enter__(self):
        try:
            command = ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1',
                       '--manifest-path', str(self.manifest)]
            self.record['metadata_command'] = command
            self.save()
            with (self.output / 'cache-metadata.json').open('x') as stdout, \
                 (self.output / 'cache-metadata.log').open('x') as stderr:
                result = subprocess.run(command, cwd=self.root, stdout=stdout, stderr=stderr,
                                        timeout=120)
            self.record['metadata_exit_code'] = result.returncode
            if result.returncode:
                raise RuntimeError('Cargo metadata failed; see cache-metadata.log')
            metadata = json.loads((self.output / 'cache-metadata.json').read_text())
            members = set(metadata['workspace_members'])
            packages = [p for p in metadata['packages'] if p['id'] in members]
            if not packages or {p['id'] for p in packages} != members:
                raise RuntimeError('incomplete first-party workspace package selection')
            for package in packages:
                if package['source'] is not None or not Path(package['manifest_path']).resolve().is_relative_to(self.root):
                    raise RuntimeError('first-party workspace package is outside the source tree')
            self.packages = {p['id']: p['name'] for p in packages}
            names = sorted(self.packages.values())
            if len(names) != len(set(names)):
                raise RuntimeError('ambiguous first-party package names')
            self.target = Path(metadata['target_directory']).resolve()
            if self.target == self.root or self.root.is_relative_to(self.target):
                raise RuntimeError('Cargo target must not contain the source tree')
            if self.output.is_relative_to(self.target):
                raise RuntimeError('retained output must be outside the Cargo target')
            self.target.mkdir(parents=True, exist_ok=True)
            lock_path = self.target / '.kv9-retained-build.lock'
            self.lock = lock_path.open('a+b')
            try:
                fcntl.flock(self.lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise RuntimeError('another retained build owns this Cargo target') from error
            stat = os.fstat(self.lock.fileno())
            self.lock_identity = (stat.st_dev, stat.st_ino)
            self.record.update(target_directory=str(self.target), first_party_packages=names,
                               lock_path=str(lock_path), lock_identity=list(self.lock_identity),
                               lock_acquired_ns=time.time_ns())
            self.env = dict(os.environ, CARGO_TARGET_DIR=str(self.target))
            # Explicit -p selection preserves third-party cached dependencies.
            command = ['cargo', 'clean', '--locked', '--manifest-path', str(self.manifest),
                       '--target-dir', str(self.target), '--profile',
                       'release' if self.release else 'dev']
            for name in names:
                command.extend(['-p', name])
            with (self.output / 'cache-clean.stdout').open('x') as stdout, \
                 (self.output / 'cache-clean.stderr').open('x') as stderr:
                self.run(command, stdout=stdout, stderr=stderr)
            self.record['invalidation_complete'] = True
            self.save()
            return self
        except BaseException as error:
            self.__exit__(type(error), error, error.__traceback__)
            raise

    def require_lock(self):
        if self.lock is None or self.lock.closed:
            raise RuntimeError('retained build requires its live cache lock')
        stat = Path(self.lock.name).stat()
        if (stat.st_dev, stat.st_ino) != self.lock_identity:
            raise RuntimeError('retained-build lock file identity changed')

    def run(self, command, *, stdout, stderr, timeout=900):
        self.require_lock()
        row = {'argv': command, 'started_ns': time.time_ns()}
        self.record['commands'].append(row)
        self.save()
        try:
            # Passing the descriptor also keeps ownership while Cargo is alive if
            # the Python owner is killed. Never explicitly unlock that descriptor.
            result = subprocess.run(command, cwd=self.root, env=self.env, stdout=stdout,
                                    stderr=stderr, timeout=timeout,
                                    pass_fds=(self.lock.fileno(),))
            row['exit_code'] = result.returncode
            if result.returncode:
                raise RuntimeError('Cargo command failed; see retained command logs')
            self.require_lock()
            return result
        except BaseException as error:
            row['failure'] = repr(error)
            raise
        finally:
            row['ended_ns'] = time.time_ns()
            self.save()

    def check_artifacts(self, path):
        """First observation of a cleaned project unit must be a real compile."""
        self.require_lock()
        rows = [json.loads(line) for line in Path(path).read_text().splitlines()]
        if not rows or rows[-1] != {'reason': 'build-finished', 'success': True}:
            raise RuntimeError('Cargo build did not finish successfully')
        found = False
        for row in rows:
            if row.get('reason') != 'compiler-artifact' or row['package_id'] not in self.packages:
                continue
            found = True
            key = json.dumps([row['package_id'], row['target'], row['profile'], row['features']], sort_keys=True)
            evidence = {k: row[k] for k in ('package_id', 'target', 'profile', 'features', 'fresh', 'filenames')}
            evidence.update(cargo_json=str(path), seen_after_invalidation=key in self.compiled)
            self.record['first_party_artifacts'].append(evidence)
            self.save()
            if row['fresh'] and key not in self.compiled:
                raise RuntimeError('first-party unit remained fresh after explicit invalidation')
            self.compiled.add(key)
        if not found:
            raise RuntimeError('Cargo graph contains no selected first-party artifacts')

    def __exit__(self, kind, error, traceback):
        if error is not None:
            self.record['failure'] = repr(error)
        self.record['complete'] = error is None
        self.record['ended_ns'] = time.time_ns()
        try:
            self.save()
        finally:
            if self.lock is not None:
                self.lock.close()
        return False
