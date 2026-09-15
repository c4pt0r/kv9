"""Read original archived reporting without extraction or codec execution."""
import gzip
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


archive = ROOT / 'original-evidence.tar.gz'
inventory = ROOT / 'input-inventory.json'
require(sha(archive) == 'e10fc538ef408894f1c5f0abc35b717fb7aa79aa9a43b4e50be15308114f312f', 'archive hash')
require(sha(inventory) == '0e70579d701252ca7dc2eb1547db00303cc4fc423788474fbb7eb1ca87426ad3', 'inventory hash')
entries = json.loads(inventory.read_bytes())
expected = {entry['name']: entry for entry in entries}
require(len(entries) == len(expected) == 10491, 'member population')
names = {'preparation/group-plan.json', 'preparation/selections/000.json',
         'root/group-000-accepted-summary.json', 'root/group-000-finish-tool-terminal.json',
         'root/controls-first/result.json', 'root/controls-first/delta-controls.stderr',
         'root/dispatcher-controls-first/result.json', 'root/dispatcher-controls-first/dispatcher-controls.stderr'}
phase_names = ['staged.json', 'verification-first/result.json', 'retire-1/result.json',
               'restore-1/result.json', 'readback-1/result.json', 'retire-2/result.json']
names.update('transaction/' + name for name in phase_names)
retained = {}
seen = set()
total = 0
with gzip.open(archive, 'rb') as compressed:
    with tarfile.open(fileobj=compressed, mode='r|') as tar:
        for member in tar:
            require(member.isfile() and member.name in expected and member.name not in seen, 'unexpected member')
            entry = expected[member.name]
            require(member.size == entry['bytes'] and member.size <= 16 * 1024**2, 'member bound')
            with tar.extractfile(member) as source:
                data = source.read(member.size + 1)
            require(len(data) == member.size and hashlib.sha256(data).hexdigest() == entry['sha256'], 'original bytes')
            seen.add(member.name)
            total += len(data)
            require(total <= 128 * 1024**2, 'aggregate bound')
            if member.name in names:
                retained[member.name] = data
    while compressed.read(1024**2):
        pass
require(seen == set(expected) and total == 52461095, 'complete archive readback')


def doc(name):
    return json.loads(retained[name])


group = doc('preparation/group-plan.json')
selection = doc('preparation/selections/000.json')
summary = doc('root/group-000-accepted-summary.json')
require(len(group['rows']) == 96 and sum(row['target_count'] for row in group['rows']) == 6681, 'finite group scope')
require(sum(row['eligible_target_allocated_bytes'] for row in group['rows']) == 78026149888, 'allocation ceiling')
require(summary['complete'] and summary['ordinal'] == 0 and summary['state'] == 'COLD', 'completed first group')
require(hashlib.sha256(retained['preparation/selections/000.json']).hexdigest() == summary['selection_sha256'], 'selection binding')
phases = {}
for name in phase_names:
    raw = retained['transaction/' + name]
    require(hashlib.sha256(raw).hexdigest() == summary['phase_hashes'][name], 'phase hash')
    phases[name] = json.loads(raw)
    require(phases[name]['complete'] and phases[name]['selection_sha256'] == summary['selection_sha256'], 'phase completion and selection')
terminal = doc('root/group-000-finish-tool-terminal.json')
require(terminal['exit_code'] == 0 and terminal['result_sha256'] == summary['phase_hashes']['retire-2/result.json'], 'actual finish terminal')
require(hashlib.sha256(retained['root/group-000-finish-tool-terminal.json']).hexdigest() == summary['finish_tool_sha256'], 'actual tool binding')
members = {row['id']: row for row in selection['members']}
targets = {row['target'] for row in selection['pairs']}
require(len(members) == 369 and len(targets) == 226, 'exact original population')
stage = phases['staged.json']
verified = phases['verification-first/result.json']
restored = phases['restore-1/result.json']
readback = phases['readback-1/result.json']
for result in [stage, verified, restored]:
    require(result['all_children_reaped'] and len(result['rows']) == 226 and {row['id'] for row in result['rows']} == targets, 'complete exact target rows')
for row in verified['rows']:
    require(row['decoded_to_eof'] and row['original_compressed_exact'] and row['original'] == members[row['id']]['original']
            and row['compressed'] == members[row['id']]['compressed'], 'exact original raw and encoded identity')
require(verified['staged_sha256'] == summary['phase_hashes']['staged.json'], 'stage lineage')
require(restored['verified_sha256'] == summary['phase_hashes']['verification-first/result.json'], 'restore lineage')
require(readback['restore_result_sha256'] == summary['phase_hashes']['restore-1/result.json'], 'readback lineage')
require(readback['readback_scope'] == summary['readback_scope'] == 'readback_only_current_capacity_policy_adaptation', 'honest adapted reader scope')
require(readback['readback_reader'] == selection['readback_reader'] and readback['original_reader'] == selection['legacy_reader'], 'original and adapted reader bindings')
require(readback['observed']['files'] == 369 and readback['observed']['logical_bytes'] == summary['logical_bytes'] == 11249173926
        and readback['observed']['compressed_bytes'] == summary['encoded_bytes'] == 7961430085, 'whole cohort bytes')
children = [child for row in stage['rows'] for child in row['children']]
children += verified['children'] + restored['children'] + readback['decoder_receipts']
require(len(children) == 2403 and all(child['exit_code'] == 0 and (child.get('reaped') or child.get('absent')) for child in children), 'actual codec outcomes')
require(len(summary['codec_lifetimes']) == 2403 and all(not row['same_lifetime_present'] for row in summary['codec_lifetimes']), 'recorded lifetime observations')
require(len(summary['preserved_objects']) == 143, 'preserved object population')
for index, original in enumerate(selection['metadata']):
    entry = expected[f"original-inputs/metadata/{index:03d}-{Path(original['path']).name}"]
    require(entry['bytes'] == original['bytes'] and entry['sha256'] == original['sha256'], 'original metadata unchanged')
require(sum(row['patch']['pin']['bytes'] for row in stage['rows']) == summary['patch_bytes'] == 228496269, 'actual patch total')
account = summary['accounting']
require(account['complete'] and account['cold_cohorts'] == 1 and not account['incomplete_or_restored_cohorts'], 'only first cohort completed')
net = account['target_gross_allocation_decrease_bytes'] - account['transaction_allocated_bytes'] - account['execution_allocated_bytes'] - account['preparation_allocated_bytes']
require(net == account['conservative_net_allocated_change_bytes'] == 2360422400, 'charged recovery accounting')
require(not account['target_reached'] and not account['all_selected_exhausted'] and not account['benchmark_runtime_ready']
        and not summary['new_database_benchmark'], 'pending group and benchmark scope')
for prefix, count, output in [('controls-first', 10, 'delta-controls.stderr'), ('dispatcher-controls-first', 2, 'dispatcher-controls.stderr')]:
    result = doc('root/' + prefix + '/result.json')
    text = retained['root/' + prefix + '/' + output].decode()
    require(result['complete'] and result['controls'] == count and 'Ran ' + str(count) + ' tests' in text and text.rstrip().endswith('OK'), 'actual focused controls')
print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=total, focused_controls=12,
                     completed_cohorts=1, selected_cohorts=96, exact_restored_objects=226,
                     whole_cohort_objects=369, codec_lifetimes=2403, net_recovery_bytes=net,
                     scope='Portable original reporting verification; no codec or database rerun.')))
