#!/usr/bin/env python3
"""Copy the explicitly selected reporting evidence; never run the runtime/auditor."""
import gzip
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PREP = Path('/tmp/kv9-read-credit-crc-performance-preparation')
REPAIR = Path('/tmp/kv9-read-credit-crc-performance-audit-repair-first')
ROOT = Path('/tmp/kv9-read-credit-crc-performance-root-preparation')
RUN = Path('/tmp/kv9-read-credit-crc-performance-timing-first')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    return json.loads(path.read_text())


def main():
    assert __debug__
    assert not (HERE / 'index.json').exists(), 'preserve the original package'
    audit = read(REPAIR / 'results-first/audit.json')
    accepted_inventory = read(REPAIR / 'results-first/input-inventory.json')
    stats_inputs = read(REPAIR / 'statistics/statistics-inputs-first.json')
    frozen = read(PREP / 'inventory-final-first.json')
    external = read(PREP / 'external-inputs-first.json')
    assert audit['complete'] and audit['matched_diagnostic_accepted']
    assert len(audit['cases']) == 72 and audit['parent_confirmed_session'] == 34148
    selection = {}

    def select(source, relative):
        source = Path(source)
        assert source.is_file() and not source.is_symlink(), source
        assert source.stat().st_size < 12 * 1024 * 1024, source
        previous = selection.setdefault(str(source), relative)
        assert previous == relative, (source, previous, relative)

    for case in audit['cases']:
        directory = Path(case['directory'])
        select(directory / 'run/report.json', f'reports/{directory.name}/report.json')
        select(directory / 'resource-coverage.json', f'reports/{directory.name}/resource-coverage.json')
    for name in frozen:
        source = Path(name)
        assert source.is_relative_to(PREP), source
        select(source, 'preparation/' + str(source.relative_to(PREP)))
    for name in ['READINESS.json', 'README.md', 'inventory-final-first.json',
                 'external-inputs-first.json', 'roles-final-first.json',
                 'final-pin-binding-second.json', 'root-release-rebuilt-readback.json',
                 'pin-readback-first-failure.json', 'finalize-pins-first.py',
                 'complete-pin-readback-second.py', 'contracts-final-first.json']:
        select(PREP / name, 'preparation/' + name)
    for directory, relative in [(PREP / 'results-first', 'failed-audit'),
                                (REPAIR, 'repair'), (REPAIR / 'results-first', 'audit'),
                                (REPAIR / 'statistics', 'statistics'), (ROOT, 'root'),
                                (RUN, 'outer')]:
        for source in sorted(directory.iterdir()):
            if source.is_file():
                select(source, relative + '/' + source.name)
    for name in ['driver.py', 'plan.json', 'matrix.json', 'isolation-snapshot.json']:
        select(RUN / 'cohorts' / name, 'run/' + name)
    excluded_binaries = []
    for name, binding in external.items():
        source = Path(name)
        if source.name in {'kv9', 'kv9-batch-benchmark', 'kv9-redis-batch-reference'}:
            excluded_binaries.append(dict(source=name, **binding))
            continue
        select(source, 'external/' + str(source.relative_to('/tmp')))

    rows = []
    matches = dict(audit=0, statistics=0, frozen_preparation=0, frozen_external=0)
    for source_name, relative in selection.items():
        source = Path(source_name)
        data = source.read_bytes()
        binding = dict(bytes=len(data), sha256=sha(data))
        for label, inventory in [('audit', accepted_inventory),
                                 ('statistics', stats_inputs['files']),
                                 ('frozen_preparation', frozen), ('frozen_external', external)]:
            if source_name in inventory:
                assert binding == inventory[source_name], (label, source_name)
                matches[label] += 1
        compressed = source.name == 'report.json' or len(data) > 128 * 1024
        stored = gzip.compress(data, compresslevel=9, mtime=0) if compressed else data
        relative += '.gz' if compressed else ''
        destination = HERE / relative
        assert destination.is_relative_to(HERE)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open('xb') as stream:
            stream.write(stored)
        actual = destination.read_bytes()
        assert actual == stored
        assert (gzip.decompress(actual) if compressed else actual) == data
        assert source.read_bytes() == data, 'original changed while copying'
        rows.append(dict(source=source_name, path=relative,
                         encoding='gzip' if compressed else 'identity',
                         decoded_bytes=len(data), decoded_sha256=sha(data),
                         stored_bytes=len(stored), stored_sha256=sha(stored)))
    assert len({r['path'] for r in rows}) == len(rows)
    assert matches['statistics'] == 144 and matches['frozen_preparation'] == len(frozen)
    assert matches['frozen_external'] + len(excluded_binaries) == len(external)
    assert set(stats_inputs['files']).issubset(selection)
    alias_source = REPAIR / 'statistics/READOUT.md'
    alias_data = alias_source.read_bytes()
    with (HERE / 'READOUT.md').open('xb') as stream:
        stream.write(alias_data)
    assert (HERE / 'READOUT.md').read_bytes() == alias_data
    aliases = [dict(source=str(alias_source), path='READOUT.md', bytes=len(alias_data), sha256=sha(alias_data))]
    generated = []
    for name in ['package.py', 'verify.py', 'README.md']:
        path = HERE / name
        generated.append(dict(path=name, bytes=path.stat().st_size, sha256=sha(path.read_bytes())))
    index = dict(
        version=1, protocol_id=audit['protocol_id'],
        baseline_revision='ca0002c7f8e9ee6f595efcc9f4151085ccce87cb',
        candidate_revision='de37c71009e8199859b931e0037818f490e8d3f6',
        client_revision='0be806d9671e2c50701a64aa7889c8859b7648ba',
        scope='Portable reporting evidence with original byte identities and all report/statistics inputs. This is not the complete runtime archive or a standalone replay of the acceptance audit.',
        cohorts=72, coverage_files=72, copies=len(rows),
        compressed_reports=sum(r['source'].endswith('/run/report.json') and r['encoding']=='gzip' for r in rows),
        compressed_files=sum(r['encoding']=='gzip' for r in rows),
        decoded_bytes=sum(r['decoded_bytes'] for r in rows), stored_bytes=sum(r['stored_bytes'] for r in rows),
        verification=dict(all_copies_read_back=True, all_decoded_bytes_equal_original=True,
                          all_original_bytes_unchanged_after_copy=True, accepted_bindings=matches),
        local_runtime_scope=dict(retained_files=audit['retained_files'], retained_bytes=audit['retained_bytes'],
            inventory_original=str(REPAIR / 'results-first/input-inventory.json'),
            inventory_sha256=sha((REPAIR / 'results-first/input-inventory.json').read_bytes()),
            omitted='WAL/data files, retained executables, full source trees, detailed resource/drain/process observations and host-wide dumps. These remain local; original accepted inventory hashes are included.'),
        excluded_binary_bindings=excluded_binaries,
        retained_failures=['root/audit-first.log', 'root/audit-terminal-first.json',
                           'failed-audit/audit.json', 'preparation/pin-readback-first-failure.json'],
        files=rows, generated_files=generated, aliases=aliases)
    with (HERE / 'index.json').open('x') as stream:
        json.dump(index, stream, indent=2, sort_keys=True)
        stream.write('\n')
    print(json.dumps(dict(copies=len(rows), compressed_reports=index['compressed_reports'],
        decoded_bytes=index['decoded_bytes'], stored_bytes=index['stored_bytes'],
        index_sha256=sha((HERE / 'index.json').read_bytes()), accepted_bindings=matches), sort_keys=True))


if __name__ == '__main__':
    main()
