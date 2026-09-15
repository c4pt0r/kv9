"""Verify every archived reporting byte; this does not execute a WAL restoration."""
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


inventory = ROOT / 'input-inventory.json'
archive = ROOT / 'original-evidence.tar.gz'
require(sha(inventory) == '65a9cb88f8edbb8ace77b73d25fde593339aae127a62d755d8c614fa2f214153', 'inventory hash')
require(sha(archive) == 'd8d168c81bff40d4ffae29c6cd30ff8c33ab7523dcc6c70633c7015cb8471a1c', 'archive hash')
entries = json.loads(inventory.read_bytes())
expected = {entry['name']: entry for entry in entries}
require(len(entries) == len(expected) == 3759, 'exact member population')
selected = {'preparation/selection.json', 'root/accepted-summary.json',
            'root/control-1.stderr', 'root/control-2.stderr', 'root/controls-tool-terminal.json'}
selected.update('transaction/' + name for name in ['staged.json', 'verification-first/result.json',
                'restore-1/result.json', 'legacy-1/result.json', 'retire-1/result.json', 'retire-2/result.json'])
retained = {}
seen = set()
decoded = 0
with gzip.open(archive, 'rb') as stream:
    with tarfile.open(fileobj=stream, mode='r|') as tar:
        for member in tar:
            require(member.isfile() and member.name in expected and member.name not in seen, 'unexpected archive entry')
            entry = expected[member.name]
            require(member.size == entry['bytes'] and member.size <= 16 * 1024**2, 'member bound')
            with tar.extractfile(member) as source:
                data = source.read(member.size + 1)
            require(len(data) == member.size and hashlib.sha256(data).hexdigest() == entry['sha256'], 'original member bytes')
            decoded += len(data)
            require(decoded <= 64 * 1024**2, 'aggregate bound')
            seen.add(member.name)
            if member.name in selected:
                retained[member.name] = data
    while stream.read(1024**2):
        pass
require(seen == set(expected) and decoded == 18815272, 'complete archive readback')


def doc(name):
    return json.loads(retained[name])


selection = doc('preparation/selection.json')
summary = doc('root/accepted-summary.json')
require(summary['complete'] and summary['state'] == 'COLD', 'completed migration')
require(summary['selection_sha256'] == hashlib.sha256(retained['preparation/selection.json']).hexdigest(), 'selection binding')
for name, digest in summary['phases'].items():
    require(hashlib.sha256(retained['transaction/' + name]).hexdigest() == digest, 'phase binding')
stage = doc('transaction/staged.json')
verified = doc('transaction/verification-first/result.json')
restored = doc('transaction/restore-1/result.json')
legacy = doc('transaction/legacy-1/result.json')
cold = doc('transaction/retire-2/result.json')
require(all(result['complete'] for result in [stage, verified, restored, legacy, cold]), 'complete phase results')
targets = {row['target'] for row in selection['pairs']}
require(len(targets) == 80 and len(selection['members']) == 150, 'original selection scope')
members = {row['id']: row for row in selection['members']}
for result in [stage, verified, restored]:
    require(len(result['rows']) == 80 and {row['id'] for row in result['rows']} == targets, 'exact target population')
for row in verified['rows']:
    require(row['decoded_to_eof'] and row['original_compressed_exact'], 'independent complete reconstruction')
    require(row['original'] == members[row['id']]['original'] and row['compressed'] == members[row['id']]['compressed'], 'original raw and encoded identities')
require(legacy['unchanged_reader'] == selection['legacy_reader'], 'original reader binding')
require(legacy['observed']['files'] == 150 and legacy['observed']['logical_bytes'] == 4015670348
        and legacy['observed']['compressed_bytes'] == 2838288732, 'whole-cohort reader scope')
require(legacy['restore_result_sha256'] == summary['phases']['restore-1/result.json'], 'actual restoration binding')
children = [child for row in stage['rows'] for child in row['children']]
children += verified['children'] + restored['children'] + legacy['decoder_receipts']
require(len(children) == 870 and all(child['exit_code'] == 0 and (child.get('reaped') or child.get('absent')) for child in children), 'actual codec outcomes')
require(len(summary['lifetimes']) == 870 and all(not row['same_lifetime_present'] for row in summary['lifetimes']), 'final lifetime observations')
require(len(summary['preserved_objects']) == 70 and summary['original_metadata_unchanged'], 'preserved evidence scope')
require(sum(row['patch']['pin']['bytes'] for row in stage['rows']) == summary['patch_encoded_bytes'] == 1397799, 'patch size')
accounting = summary['accounting']
require(accounting['current_target_allocated_bytes'] == 0, 'final target absence')
require(accounting['original_target_allocated_bytes'] - accounting['transaction_allocated_bytes']
        == accounting['conservative_net_allocated_change_bytes'] == 924991488, 'transaction net allocation')
require(summary['charged_decoded_bytes'] == 12036960356 and not summary['new_database_benchmark_run'], 'decode and benchmark scope')
require(doc('root/controls-tool-terminal.json')['exit_code'] == 0, 'control terminal')
for name, count in [('control-1', 18), ('control-2', 1)]:
    output = retained['root/' + name + '.stderr'].decode()
    require('Ran ' + str(count) + ' test' in output and output.rstrip().endswith('OK'), 'actual control population')
print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded, controls=19,
                     exact_restored_objects=80, whole_cohort_objects=150, codec_lifetimes=870,
                     transaction_net_recovery_bytes=924991488, scope='Portable original reporting readback; no codec or runtime rerun.')))
