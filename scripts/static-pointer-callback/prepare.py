#!/usr/bin/env python3
"""Prepare isolated, checksum-verified dependency workspaces; do not edit Cargo's registry."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib

REPO = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True,
                        help='Cargo registry cache directory containing the exact .crate files')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    lock = tomllib.loads((REPO / 'Cargo.lock').read_text())
    contract = json.loads((REPO / 'proofs/lean/static-pointer-callback/source-contract.json').read_text())
    records = []
    for name, version, arms in [('archery', '1.2.3', ('baseline', 'candidate')),
                                ('rpds', '1.2.1', ('candidate',))]:
        package = [p for p in lock['package'] if p['name'] == name and p['version'] == version]
        assert len(package) == 1
        archive = args.cache / f'{name}-{version}.crate'
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        assert checksum == package[0]['checksum'], f'Archive checksum mismatch: {archive}'
        for arm in arms:
            destination = output / f'{arm}-{name}'
            destination.mkdir()
            with tarfile.open(archive) as tar:
                for member in tar.getmembers():
                    relative = Path(member.name).relative_to(f'{name}-{version}')
                    assert '..' not in relative.parts and not relative.is_absolute()
                    target = destination / relative
                    if member.isdir():
                        target.mkdir(parents=True, exist_ok=True)
                    else:
                        assert member.isfile(), 'Archive links/devices are outside this preparation contract'
                        target.parent.mkdir(parents=True, exist_ok=True)
                        with tar.extractfile(member) as source:
                            target.write_bytes(source.read())
            with (destination / 'Cargo.toml').open('a') as manifest:
                manifest.write('\n[workspace]\n')
                if name == 'rpds':
                    manifest.write('\n[patch.crates-io]\narchery = { path = "../candidate-archery" }\n')
            shutil.copyfile(REPO / 'Cargo.lock', destination / 'Cargo.lock')
        records.append({'name': name, 'version': version, 'archive_sha256': checksum})
    subprocess.run(['patch', '--batch', '--fuzz=0', '-p1', '-i', str(Path(__file__).with_name('archery.patch'))],
                   cwd=output / 'candidate-archery', check=True)
    for arm in ('baseline', 'candidate'):
        source = output / f'{arm}-archery'
        guard = source / 'src/shared_pointer/kind/erased_ptr.rs'
        with guard.open('a') as file:
            file.write('\n#[cfg(test)]\nmod callback_tests;\n')
        tests = guard.with_suffix('') / 'callback_tests.rs'
        tests.parent.mkdir(exist_ok=False)
        shutil.copyfile(Path(__file__).with_name('callback_tests.rs'), tests)
        for relative, expected in contract['sources'][arm].items():
            assert hashlib.sha256((source / relative).read_bytes()).hexdigest() == expected, relative
    (output / 'preparation.json').write_text(json.dumps({'complete': True, 'archives': records}, indent=2) + '\n')
    print(json.dumps({'complete': True, 'output': str(output)}))


if __name__ == '__main__':
    main()
