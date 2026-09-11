"""Finite preparation checks only: no child servers/clients or measurement."""
import ast
import copy
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace

p = Path(__file__).resolve().parent
ast.parse((p/'run.py').read_text())
spec = importlib.util.spec_from_file_location('prepared_smoke', p/'run.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
sys.path.insert(0, str(r.CLIENT_SOURCE/'scripts'))
import batch_benchmark_report as native
import redis_batch_report as redis
legacy = r.module('retained_check_helpers', r.LEGACY_PATH)
resp = r.module('retained_check_resp', r.RESP_PATH)
protocol = r.read(p/'protocol.json')
args = {'client_revision': protocol['clients']['native']['revision']}
for role, pin in protocol['clients'].items():
    args[role+'_build'] = Path(pin['build'])
    args[role+'_binary_sha256'] = pin['binary_sha256']
    args[role+'_manifest_sha256'] = pin['manifest_sha256']
bound = r.bind_inputs(SimpleNamespace(**args), legacy, native)
r.require(protocol['cases'] == r.cases() and len(r.cases()) == 6, 'case inventory differs')
checked, rejected = [], []
for case in r.cases():
    n, red = r.configs(case, [{'node_id': 1, 'address': '127.0.0.1:20160'}], 1, '127.0.0.1:6379')
    native.config_check(n); redis.config_check(red); redis.paired_configuration(red, n)
    checked.append(case['name'])
    bad = dict(red, write_api='mset' if red['write_api'] == 'set' else 'set')
    try:
        redis.paired_configuration(bad, n)
    except ValueError:
        rejected.append(case['name']+'-wrong-write-pair')
    else:
        raise AssertionError('wrong write pairing accepted')

# The smoke's additional label/attempt predicate gets a selected actual point
# response population, then independent corruptions. No raw-runtime claim.
c, _ = r.configs(r.cases()[0], [{'node_id': 1, 'address': '127.0.0.1:20160'}], 1, '127.0.0.1:6379')
op = lambda n: dict(populations=[dict(calls=n,input_items=n)]+[dict(calls=0) for _ in range(4)],
                    reasons=[n]+[0]*11,attempts=[dict(raw=dict(count=k)) for k in (n,0,0)])
report = dict(measured_issued=1,metrics={'measurement':dict(operations=['get','put'],statistics=[op(0),op(1)])})
r.check_populations(report,c,False,resp)
for label, modify in [('batch-label',lambda x:x['metrics']['measurement'].update(operations=['batch_get','batch_put'])),
    ('extra-attempt',lambda x:x['metrics']['measurement']['statistics'][1]['attempts'][0]['raw'].update(count=2))]:
    bad=copy.deepcopy(report);modify(bad)
    try:
        r.check_populations(bad,c,False,resp)
    except ValueError:
        rejected.append(label)
    else:
        raise AssertionError('corruption accepted: '+label)
r.save(p/'contract-first.json',dict(complete=True,syntax=True,paired_configs=checked,rejections=rejected,
    source_counts={name:len(role['source']['sources']) for name,role in bound.items()},
    role_binary_hashes={name:role['binary_sha256'] for name,role in bound.items()},
    scope='Finite configuration, frozen build/source and selected population checks; no runtime execution'))
print('PASS: six config pairs, eight negative controls, all frozen role sources/builds')
