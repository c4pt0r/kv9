"""Pure offline controls, authored for later release; never launch a process."""
import copy
import unittest
import active_prefix as prefix


def valid_summary():
    calls = []
    for slot in range(prefix.CONFIG['calls']):
        calls.append(dict(slot=slot, command=['/retained/kv9', 'client', 'raw-get'],
                          identity_captured=True, exited=True, absent=True, outcome='not_found',
                          exit_code=0, completed_monotonic_ns=1100, deadline_monotonic_ns=1200,
                          stdout='found=false\n', stderr=''))
    return dict(configuration=copy.deepcopy(prefix.CONFIG), retries=0, start_monotonic_ns=0,
                dispatch_end_monotonic_ns=1_000_000_000, end_monotonic_ns=1_100_000_000,
                calls=calls, max_live=8, voter_lifetimes_unchanged=True,
                executable_unchanged=True, cleanup_errors=[])


class PrefixContracts(unittest.TestCase):
    def rejects(self, change):
        summary = valid_summary()
        change(summary)
        with self.assertRaises(ValueError):
            prefix.validate_summary(summary)

    def test_valid_complete_prefix(self):
        prefix.validate_summary(valid_summary())

    def test_command_is_raw_get_and_key_guarded(self):
        argv = prefix.command('/retained/kv9', '127.0.0.1:20160', 100, b'__profile_prefix__:prfbat')
        self.assertEqual(argv[1:3], ['client', 'raw-get'])
        self.assertEqual(argv[-2:], ['--key-hex', b'__profile_prefix__:prfbat'.hex()])
        with self.assertRaises(ValueError):
            prefix.command('/retained/kv9', '127.0.0.1:20160', 100, b'prfbat:measured')
        self.rejects(lambda s: s['calls'][0]['command'].__setitem__(2, 'raw-put'))

    def test_missing_or_duplicate_calls_rejected(self):
        self.rejects(lambda s: s['calls'].pop())
        self.rejects(lambda s: s['calls'][1].__setitem__('slot', 0))

    def test_live_or_unbound_child_rejected(self):
        for field in ['exited', 'absent', 'identity_captured']:
            self.rejects(lambda s, field=field: s['calls'][0].__setitem__(field, False))

    def test_read_failure_value_hit_and_deadline_rejected(self):
        for field, value in [('outcome', 'deadline'), ('exit_code', 1), ('stdout', 'value_hex=6162\n'), ('stderr', 'not_leader=true\n')]:
            self.rejects(lambda s, field=field, value=value: s['calls'][0].__setitem__(field, value))
        self.rejects(lambda s: s['calls'][0].__setitem__('completed_monotonic_ns', 1201))

    def test_bounds_replay_and_cleanup_rejected(self):
        for field, value in [('retries', 1), ('max_live', 9), ('dispatch_end_monotonic_ns', 999_999_999), ('end_monotonic_ns', 4_000_000_001)]:
            self.rejects(lambda s, field=field, value=value: s.__setitem__(field, value))
        self.rejects(lambda s: s['cleanup_errors'].append({'pid': 123, 'error': 'not reaped'}))
        self.rejects(lambda s: s['configuration'].__setitem__('call_deadline_ms', 30000))

    def test_voter_and_executable_binding_rejected(self):
        self.rejects(lambda s: s.__setitem__('voter_lifetimes_unchanged', False))
        self.rejects(lambda s: s.__setitem__('executable_unchanged', False))
        sample = dict(pid=1, start_ticks=2, boot_id='boot', executable_device=3, executable_inode=4, executable_bytes=5)
        for key in sample:
            changed = dict(sample, **{key: 'different'})
            self.assertFalse(prefix.same_lifetime(sample, changed))


if __name__ == '__main__':
    unittest.main()
