"""Thin original-step dispatcher: original retirement/restore, corrected readback, final COLD.
Initial actual authorization is ordinal013 only. Root owns any later campaign release.
"""
import fcntl
import signal
import sys
from authority import *

spec = importlib.util.spec_from_file_location('original_dispatcher', OLD / 'run.py')
dispatcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dispatcher)


def main():
    ap = repair_args(argparse.ArgumentParser())
    ap.add_argument('--ordinal', type=int, required=True)
    ap.add_argument('--mode', choices=['finish', 'restore-final', 'readback-final'], required=True)
    ap.add_argument('--code-pins-sha256', required=True)
    ap.add_argument('--release-verified-sha256', required=True)
    a = ap.parse_args()
    need(13 <= a.ordinal <= 95, 'finite original ordinal 13..95')
    source_authority(a)
    group = js(OLD / 'group-plan.json')
    row = group['rows'][a.ordinal]
    need(row['ordinal'] == a.ordinal, 'original plan ordinal')
    a.selection = row['selection']['path']
    a.selection_sha256 = row['selection']['sha256']
    a.retained_toolchain_sha256 = None
    s = load(a)
    root = Path(s['root'])
    folder = dispatcher.EXECUTION / f'{a.ordinal:03d}'
    need(folder.is_dir(), 'original verified dispatcher folder required')
    lock_fd = os.open(dispatcher.EXECUTION / 'serial.lock', os.O_RDWR | os.O_NOFOLLOW)
    fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    inherited_guard = dispatcher.guard
    def guard(start=None):
        free = inherited_guard(start)
        extra = external_allocation(root)
        need(common.allocated(root, s['limits']['metadata_bytes']) + s['limits']['external_restored_target_allowance_bytes'] + extra <= s['limits']['tx_allocated_bytes'], 'combined original transaction/repair allocation cap')
        return free
    dispatcher.guard = guard
    def interrupted(signum, frame):
        raise InterruptedError('reconciliation signal ' + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        guard()
        vp = root / 'verification-first/result.json'
        need(digest(vp)['sha256'] == a.release_verified_sha256, 'actual independently released verifier pin')
        same = dict(selection_sha256=a.selection_sha256, code_pins_sha256=CODE_SHA)
        v = dispatcher.validate_done(js(folder / 'verify-complete.json'), vp, dict(same, state='VERIFIED'))
        base = ['--selection', a.selection, '--selection-sha256', a.selection_sha256, '--code-pins-sha256', CODE_SHA]
        tool = ['--retained-toolchain-sha256', v['toolchain_sha256']]
        release = ['--verified-result', str(vp), '--release-verified-sha256', a.release_verified_sha256]
        repair = ['--preparation-inventory-sha256', a.preparation_inventory_sha256, '--reader-manifest', a.reader_manifest, '--reader-manifest-sha256', a.reader_manifest_sha256]
        binding = dict(selection_sha256=a.selection_sha256, verified_sha256=a.release_verified_sha256)
        def oldargv(*tail):
            return [sys.executable, str(OLD / 'transition.py'), *base, *tail]
        if a.mode != 'finish':
            dispatcher.validate_done(js(folder / 'restore-1-complete.json'), root / 'restore-1/result.json', dict(binding, state='RESTORED'))
            a.restore_result_sha256 = digest(root / 'restore-1/result.json')['sha256']
            first_readback = root / 'readback-1-repaired-first/result.json'
            rr = dispatcher.validate_done(js(folder / 'readback-1-repaired-complete.json'), first_readback, dict(selection_sha256=a.selection_sha256, restore_result_sha256=a.restore_result_sha256, preparation_inventory_sha256=a.preparation_inventory_sha256, **reader_binding(s)))
            repaired_reader_ok(rr, s, a)
            a.readback_result_sha256 = digest(first_readback)['sha256']
            prior = reconciliation_binding(s, a)
            dispatcher.validate_done(js(folder / 'retire-2-complete.json'), root / 'retire-2/result.json', dict(binding, state='COLD', cycle=2, reconciliation=prior))
            if a.mode == 'restore-final':
                need(guard() >= 8 * GIB + s['limits']['external_restored_target_allowance_bytes'] + s['limits']['scratch_reserve_bytes'] + 16 * MIB + external_allocation(root), 'fresh complete restore plus scratch/metadata reservation')
                result_path = root / 'restore-2/result.json'
                dispatcher.step(folder, 'restore-2', [sys.executable, str(OWN / 'transition_repaired.py'), *base, *repair, 'restore', '--cycle', '2', *release, *tool, '--restore-result-sha256', a.restore_result_sha256, '--readback-result-sha256', a.readback_result_sha256], result_path, dict(binding, state='RESTORED', reconciliation=prior))
            else:
                dispatcher.validate_done(js(folder / 'restore-2-complete.json'), root / 'restore-2/result.json', dict(binding, state='RESTORED', reconciliation=prior))
                restore2 = digest(root / 'restore-2/result.json')['sha256']
                result_path = root / 'readback-2-repaired-first/result.json'
                final = dispatcher.step(folder, 'readback-2-repaired', [sys.executable, str(OWN / 'readback_repaired.py'), *base, *repair, '--cycle', '2', '--restore-result-sha256', restore2], result_path, dict(selection_sha256=a.selection_sha256, restore_result_sha256=restore2, preparation_inventory_sha256=a.preparation_inventory_sha256, **reader_binding(s)))
                a.restore_result_sha256 = restore2
                repaired_reader_ok(final, s, a)
            guard()
            print(json.dumps(dict(complete=True, mode=a.mode, ordinal=a.ordinal, result_path=str(result_path), result_sha256=digest(result_path)['sha256'], first_cycle_reconciliation=prior, accounting=accounting(s), no_third_retirement_supported=True)))
            return 0
        dispatcher.step(folder, 'retire-1', oldargv('retire', '--cycle', '1', *release, *tool), root / 'retire-1/result.json', dict(binding, state='COLD', cycle=1))
        dispatcher.step(folder, 'restore-1', oldargv('restore', '--cycle', '1', *release, *tool), root / 'restore-1/result.json', dict(binding, state='RESTORED'))
        a.restore_result_sha256 = digest(root / 'restore-1/result.json')['sha256']
        readback = root / 'readback-1-repaired-first/result.json'
        reader_expected = dict(selection_sha256=a.selection_sha256, restore_result_sha256=a.restore_result_sha256, preparation_inventory_sha256=a.preparation_inventory_sha256, **reader_binding(s))
        rr = dispatcher.step(folder, 'readback-1-repaired', [sys.executable, str(OWN / 'readback_repaired.py'), *base, *repair, '--cycle', '1', '--restore-result-sha256', a.restore_result_sha256], readback, reader_expected)
        repaired_reader_ok(rr, s, a)
        a.readback_result_sha256 = digest(readback)['sha256']
        cold_expected = dict(binding, state='COLD', cycle=2, reconciliation=reconciliation_binding(s, a))
        dispatcher.step(folder, 'retire-2', [sys.executable, str(OWN / 'transition_repaired.py'), *base, *repair, 'retire', '--cycle', '2', *release, *tool, '--restore-result-sha256', a.restore_result_sha256, '--readback-result-sha256', a.readback_result_sha256], root / 'retire-2/result.json', cold_expected)
        guard()
        status = dispatcher.account(group)
        extra = external_allocation(root)
        status['repair_external_allocated_bytes'] = extra
        status['conservative_net_allocated_change_bytes'] -= extra
        answer = dict(complete=True, mode='finish', ordinal=a.ordinal, state='COLD', selection_sha256=a.selection_sha256, verified_sha256=a.release_verified_sha256, restore_sha256=a.restore_result_sha256, readback_path=str(readback), readback_sha256=a.readback_result_sha256, cold_sha256=digest(root / 'retire-2/result.json')['sha256'], reconciliation=reconciliation_binding(s, a), accounting=status)
        durable_json(folder / ('reconciliation-complete-' + str(time.time_ns()) + '.json'), answer)
        print(json.dumps(answer))
        return 0
    finally:
        dispatcher.guard = inherited_guard
        os.close(lock_fd)


if __name__ == '__main__':
    sys.exit(main())
