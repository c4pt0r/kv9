"""Finite corrected-reader finish authority; original source and selections stay immutable."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys

OWN = Path(__file__).resolve().parent
OLD = Path('/tmp/kv9-cross-voter-multicohort-migration-preparation-20260915-first')
sys.path.insert(0, str(OLD))
import support as original
import common
from support import *

MANIFEST = Path('/tmp/kv9-retention-pid-identity-repair-20260915-first/reader-bindings.json')
MANIFEST_SHA = '3269e44abe84555e70ad1f07e86ea32e057d828515888b23e6663c4e4b724386'
CODE_SHA = '66ac9d2f4e5fa8fc59f4fe77ca158ebba8fa40d183ed71b5ff1a6deab1920d7e'
SELECTION_SHA = '66bf03896100ca431aff0ae802708b422224267e585245ccbbbcd58658fb0ff4'
VERIFIED_SHA = '6b17d3766c740a223627b69d2161d86cc7edb7672864370a2a32616d1d686099'
RESTORE_SHA = '6b15909ae5c84acd148535aa9ca3de9139f679969de1b26d05fc0cdbccf9a5ce'
FAILURE_SHA = 'a9fedebaf08dbeb02e1733d90c9602401027be7e543525e82f350c771b77970d'
SCOPE = 'producer_lifetime_identity_repair_of_original_selected_readback_reader'
BEFORE_DECODE = 31155519021
AFTER_DECODE = 41558085954
DECODE_CAP = 69793218560
ACTIVE = None


def repair_args(ap):
    ap.add_argument('--preparation-inventory-sha256', required=True)
    ap.add_argument('--reader-manifest', required=True)
    ap.add_argument('--reader-manifest-sha256', required=True)
    return ap


def reader_row(manifest, s):
    need(manifest.get('schema') == 1, 'reader authority schema')
    families = {'fnv-writer': 'fnv-writer', 'frame-buffer-crc': 'frame-buffer-crc', 'published-directory': 'published-directory', 'crc-full-regression': 'crc-full-regression'}
    need(s['screen'] in families, 'unknown original reader family')
    family = families[s['screen']]
    rows = [r for r in manifest.get('rows', []) if r.get('family') == family]
    need(len(rows) == 1, 'exact single family reader authority')
    r = rows[0]
    need(r['capacity_policy_unchanged'] is True, 'reader changes storage policy')
    need(r['original_campaign_reader'] == s['legacy_reader'], 'original reader authority mismatch')
    need(r['prior_readback_reader'] == s['readback_reader'], 'prior reader authority mismatch')
    need(r['corrected_reader']['path'] == str(MANIFEST.parent / ('readers/' + family + '.py')), 'corrected reader path')
    return r


def source_authority(a):
    need(str(a.reader_manifest) == str(MANIFEST) and a.reader_manifest_sha256 == MANIFEST_SHA, 'explicit fixed corrected-reader manifest required')
    need(digest(OWN / 'inventory.json')['sha256'] == a.preparation_inventory_sha256, 'repair inventory changed')
    for row in js(OWN / 'inventory.json')['rows']:
        p = canonical_path(OWN / row['path'])
        need(p.is_relative_to(OWN), 'repair inventory path escape')
        check(p, row, 'repair preparation source changed')
    need(digest(MANIFEST)['sha256'] == MANIFEST_SHA, 'reader manifest changed')


def load(a):
    global ACTIVE
    source_authority(a)
    need(a.code_pins_sha256 == CODE_SHA, 'original code pins required')
    s = original.load(a)
    need(type(s['group_ordinal']) is int and 13 <= s['group_ordinal'] <= 95, 'finite unreconciled original ordinals only')
    root = Path(s['root'])
    if s['group_ordinal'] == 13:
        need(a.selection_sha256 == SELECTION_SHA and s['target_count'] == 209 and s['original_object_count'] == 344 and s['logical_cohort_bytes'] == 10402566933 and s['encoded_cohort_bytes'] == 7362155494, 'fixed ordinal013 scope')
        need(s['limits']['cumulative_decoded_bytes'] == DECODE_CAP, 'unchanged decoded cap')
        need(digest(root / 'restore-1/result.json')['sha256'] == RESTORE_SHA, 'original successful restoration changed')
        need(digest(root / 'verification-first/result.json')['sha256'] == VERIFIED_SHA, 'original successful verification changed')
        need(digest(root / 'readback-1/result.json')['sha256'] == FAILURE_SHA, 'first failed readback changed')
        failed = js(root / 'readback-1/result.json')
        need(failed['complete'] is False and failed['restore_result_sha256'] == RESTORE_SHA, 'first failure remains failed')
        reservation = js(root / 'readback-1/decode-reservation.json')
        need(reservation['charged_decoded_bytes'] == s['logical_cohort_bytes'] and reservation['previous_cumulative_decoded_bytes'] == 20752952088 and reservation['restore_result_sha256'] == RESTORE_SHA, 'first failed decode charge changed')
    else:
        need(absent(root / 'readback-1'), 'unexpected legacy readback outside explicit013 reconciliation')
    row = reader_row(js(MANIFEST), s)
    for k in ['original_campaign_reader', 'prior_readback_reader', 'preserved_source_copy', 'corrected_reader', 'diff']:
        check(row[k]['path'], row[k], 'reader authority input changed')
    ACTIVE = a
    external_allocation(root)
    return s


def effective_reader(s):
    return reader_row(js(MANIFEST), s)['corrected_reader']


def reader_binding(s):
    failed = dict(path=str(Path(s['root']) / 'readback-1/result.json'), sha256=FAILURE_SHA) if s['group_ordinal'] == 13 else None
    return dict(readback_reader=effective_reader(s), original_reader=s['legacy_reader'], original_selected_reader=s['readback_reader'], prior_readback_scope=s['readback_scope'], readback_scope=SCOPE, reader_manifest=dict(path=str(MANIFEST), sha256=MANIFEST_SHA), failed_readback=failed)


def repaired_reader_ok(doc, s, a):
    need(doc.get('complete') is True and doc.get('selection_sha256') == a.selection_sha256 and doc.get('restore_result_sha256') == a.restore_result_sha256, 'corrected readback incomplete/lineage')
    need(doc.get('preparation_inventory_sha256') == a.preparation_inventory_sha256, 'corrected readback preparation mismatch')
    for key, value in reader_binding(s).items():
        need(doc.get(key) == value, 'corrected readback authority differs: ' + key)
    need(doc.get('charged_decoded_bytes') == s['logical_cohort_bytes'], 'new full readback charge missing')
    ob = doc.get('observed', {})
    need(ob.get('files') == s['original_object_count'] and ob.get('logical_bytes') == s['logical_cohort_bytes'] and ob.get('compressed_bytes') == s['encoded_cohort_bytes'] and ob.get('decoded_all_original_bytes') is True, 'corrected full-cohort scope')
    ds = doc.get('decoder_receipts', [])
    mm = {m['path']: m for m in s['members']}
    need(len(ds) == len(mm) == s['original_object_count'] and {d['object'] for d in ds} == set(mm), 'corrected decoder object coverage')
    for d in ds:
        need(d['complete'] is True and d['exit_code'] == 0 and d['absent'] is True, 'corrected decoder outcome')
        need(dict(bytes=d['bytes'], sha256=d['sha256']) == mm[d['object']]['original'], 'corrected decoder raw identity')
        need(d['codec_sha256'] == s['codec']['sha256'], 'corrected decoder executable')
        gone(d)
    return True


def gone(row):
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    p = Path('/proc') / str(row['pid']) / 'stat'
    try:
        text = p.read_text()
    except (FileNotFoundError, ProcessLookupError):
        return
    current = int(text[text.rfind(')') + 2:].split()[19])
    need(boot != row['boot_id'] or current != row['start_ticks'], 'same recorded lifetime still present')


def external_allocation(root):
    total = 0
    for extra_root, cap in [(OWN, 32 * MIB), (MANIFEST.parent, 16 * MIB)]:
        canonical_path(extra_root)
        need(extra_root.stat().st_dev == root.stat().st_dev, 'repair metadata device differs')
        n = common.allocated(extra_root, cap)
        need(n <= cap, 'repair external allocation cap')
        total += n
    return total


def session(root, s):
    sess = original.session(root, s)
    old_guard = sess.guard
    def guarded(reserve=0):
        extra = external_allocation(root)
        sess.external_allocation_reserve = s['limits']['external_restored_target_allowance_bytes'] + extra
        r = old_guard(reserve)
        r['repair_external_allocated_bytes'] = extra
        return r
    sess.guard = guarded
    return sess


def accounting(s):
    r = original.accounting(s)
    extra = external_allocation(Path(s['root']))
    r['repair_external_allocated_bytes'] = extra
    r['conservative_net_allocated_change_bytes'] -= extra
    r['scope'] += ' Also charges this entire repair preparation/execution and corrected-reader preparation; no original charge removed.'
    return r


def charge_gate(previous, charge, limit, cycle=1):
    need(cycle in (1, 2), 'only two readback cycles')
    expected_before = BEFORE_DECODE if cycle == 1 else 48475736650
    expected_after = AFTER_DECODE if cycle == 1 else 58878303583
    need(previous == expected_before and charge == 10402566933 and limit == DECODE_CAP, 'exact retained previous/new decoded charge')
    need(previous + charge == expected_after and previous + charge <= limit, 'corrected readback cumulative cap')


def reconciliation_binding(s, a):
    return dict(ordinal=s['group_ordinal'], original_code_pins_sha256=CODE_SHA, preparation_inventory_sha256=a.preparation_inventory_sha256, restore_result_sha256=a.restore_result_sha256, corrected_readback=dict(path=str(Path(s['root']) / 'readback-1-repaired-first/result.json'), sha256=a.readback_result_sha256), **reader_binding(s))
