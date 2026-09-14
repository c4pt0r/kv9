"""Finite native48 reporting gates; the original campaign auditor remains authoritative."""
from pathlib import Path
import hashlib

HERE = Path(__file__).resolve().parent
PREP = Path('/tmp/kv9-write-crc-slicing8-full-regression-runtime-preparation-first')
RUN = Path('/tmp/kv9-write-crc-slicing8-full-regression-timing-first/cohorts')
SMOKE = Path('/tmp/kv9-write-crc-slicing8-full-regression-smoke-first')
AUDIT = PREP / 'results-first/audit.json'
PROTOCOL = 'kv9-crc-slicing8-full-native-regression-c1-c64-v1'
ROLES = ('old', 'new')
WORKLOADS = tuple(f'{w}-r{r:03d}' for w in ('point', 'batch64') for r in (0, 50, 100))
SERVER_PINS = {
    'old': dict(revision='11113f68f6a5df77da1ffb4fcec850953716ffa3',
                binary_sha256='dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc',
                manifest_sha256='582ee4f5840aa8cf04168ee8aeffe92250ed0807ee3f53aea93b25d3494b809d'),
    'new': dict(revision='e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a',
                binary_sha256='616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476',
                manifest_sha256='32cf899d55b7160f5c3aacfff24da0d145d381e5082b2a6925b94b4cedcf1463'),
}
CLIENT_REV = '0be806d9671e2c50701a64aa7889c8859b7648ba'
CLIENT_PIN = dict(binary_sha256='8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957',
                  manifest_sha256='eb8b4e1421dd4e64dcca59fcc7829926d5789611f6b3748d07c4090f634f2c4c')


def descriptors(smoke=False):
    result = []
    for repeat in range(1 if smoke else 2):
        order = [(c, w, size, rp, role) for c in (1, 64)
                 for w, size in (('point', 1), ('batch64', 64))
                 for rp in (0, 50, 100) for role in ROLES]
        if repeat:
            order.reverse()
        for c, w, size, rp, role in order:
            workload = f'{w}-r{rp:03d}'
            result.append(dict(repeat=repeat, arm=f'{role}-{workload}', server_role=role,
                workload=workload, read_api='point_get' if size == 1 else 'batch_get',
                write_api='point_put' if size == 1 else 'batch_put',
                mix={0: 'write', 50: 'mixed', 100: 'read'}[rp], target='kv9',
                shared=dict(run_id=f'p{repeat}{c:04d}', seed=71, workers=c, keys=4096,
                    batch_size=size, value_bytes=128, read_percent=rp, warmup_calls=128,
                    measure_ms=2000 if smoke else 10000, max_calls=10000000,
                    load=dict(kind='closed_loop'))))
    return result


def protocol_gate(protocol):
    assert protocol['protocol_id'] == PROTOCOL
    assert protocol['server_pins'] == SERVER_PINS, 'wrong server roles/source/build pins'
    assert protocol['client_revision'] == CLIENT_REV
    assert protocol['client_pins'] == {'client': CLIENT_PIN}
    assert protocol['timed_inventory'] == descriptors(), 'full exact timing order required'
    assert protocol['smoke_inventory'] == descriptors(True), 'full exact smoke order required'


def terminal_gate(terminal, audit_hash, inventory_hash):
    assert terminal['complete'] is True and terminal['exit_code'] == 0
    assert terminal['terminal_receipt']
    if terminal['session_id'] is None:
        assert terminal['execution_kind'] == 'direct_tool_terminal'
        assert terminal['tool_result']['exit_code'] == 0
        assert terminal['tool_result']['chunk_id'] == terminal['terminal_receipt']
    else:
        assert type(terminal['session_id']) is int and terminal['session_id'] > 0
    assert terminal['audit_path'] == str(AUDIT) and terminal['audit_sha256'] == audit_hash
    assert terminal['input_inventory_path'] == str(AUDIT.with_name('input-inventory.json'))
    assert terminal['input_inventory_sha256'] == inventory_hash


def expected_case(ordinal, desc):
    return dict(ordinal=ordinal, repeat=desc['repeat'], concurrency=desc['shared']['workers'],
        arm=desc['arm'], workload=desc['workload'], read_api=desc['read_api'],
        write_api=desc['write_api'], batch_size=desc['shared']['batch_size'],
        read_percent=desc['shared']['read_percent'],
        directory=str(RUN / f"{ordinal:03d}-{desc['arm']}-{desc['shared']['run_id']}"))


def accepted_cases(audit, protocol, session):
    protocol_gate(protocol)
    assert type(session) is int and session > 0
    assert audit['complete'] is True and audit['matched_diagnostic_accepted'] is True
    assert audit['protocol_id'] == PROTOCOL
    assert audit['parent_confirmed_session'] == session and audit['parent_confirmed_exit_code'] == 0
    assert audit['successful_arms'] == audit['expected_successful_arms'] == len(audit['cases']) == 48
    assert (audit['owned_lifetimes_exited'], audit['qualifying_drains'],
            audit['voter_writer_listener_bindings']) == (192, 144, 144)
    assert audit['performance_promotion'] is False and audit['outer_restoration']['accepted'] is True
    assert audit['smoke']['owned_lifetimes_exited'] == 96 and len(audit['smoke']['cases']) == 24
    assert audit['smoke']['timing_acceptance'] is False
    for ordinal, (case, desc) in enumerate(zip(audit['cases'], descriptors(), strict=True)):
        wanted = expected_case(ordinal, desc)
        assert {k: case[k] for k in wanted} == wanted, 'case identity/order/root differs'
    for ordinal, (case, desc) in enumerate(zip(audit['smoke']['cases'], descriptors(True), strict=True)):
        assert case['directory'] == str(SMOKE / f"{ordinal:03d}-{desc['arm']}-{desc['shared']['run_id']}")
        assert case['target'] == desc['server_role']
    retention = audit['compressed_retention']
    assert retention['independently_decoded_all_bytes'] is True and retention['resident_original_paths'] is False
    assert len(retention['cohorts']) == 48 and len(retention['smoke_byte_retention']) == 24


def matrix_gate(matrix, smoke, protocol):
    assert matrix['complete'] is True and matrix['smoke'] is False
    assert smoke['complete'] is True and smoke['smoke'] is True
    assert matrix['cohort_inventory'] == protocol['timed_inventory']
    assert smoke['cohort_inventory'] == protocol['smoke_inventory']
    assert len(matrix['attempts']) == 48 and len(smoke['attempts']) == 24
    assert matrix['role_bindings'] == smoke['role_bindings']
    roles = matrix['role_bindings']
    assert set(roles) == {'old', 'new', 'client'}, 'no historical/Redis role'
    for role, pin in SERVER_PINS.items():
        assert roles[role]['binary_sha256'] == pin['binary_sha256']
        assert roles[role]['source']['revision'] == pin['revision']
    assert roles['client']['binary_sha256'] == CLIENT_PIN['binary_sha256']
    assert roles['client']['source']['revision'] == CLIENT_REV


def report_binding(path, raw, inventory, case=None):
    path = Path(path)
    assert path.is_relative_to(RUN), 'historical/smoke performance input excluded'
    assert dict(bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest()) == inventory[str(path)]
    if case is not None:
        assert hashlib.sha256(raw).hexdigest() == case['report_sha256']
