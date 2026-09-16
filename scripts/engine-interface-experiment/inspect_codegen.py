#!/usr/bin/env python3
"""Check the actual release call boundaries before authorizing this diagnostic."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

S = Path(sys.argv[1]).resolve()
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
build = json.loads((S / 'build-summary-v1.json').read_text())
assert build['complete']
arms = {}
for arm in ('baseline', 'candidate'):
    binary = Path(build['arms'][arm]['timing']['path'])
    assert sha(binary) == build['arms'][arm]['timing']['sha256']
    symbols = subprocess.check_output(['nm', '-SC', str(binary)], text=True)
    disassembly = subprocess.check_output(['objdump', '-dC', '-Mintel', str(binary)], text=True)
    # Retained pre-review captures must describe the exact same executable.
    assert (S / f'symbols-{arm}.txt').read_text() == symbols
    assert (S / f'disassembly-{arm}.txt').read_text() == disassembly
    chosen = []
    for line in symbols.splitlines():
        parts = line.split(maxsplit=3)
        if len(parts) != 4 or not re.fullmatch('[0-9a-f]+', parts[1]):
            continue
        address, size, kind, name = parts
        if (name.endswith('::measured_read_pass') or
            name in ('<kv9_engine::mem::MemSnapshot as kv9_engine::ReadView>::get',
                     '<kv9_engine::mem::MemSnapshot as kv9_engine::ReadView>::get_resident',
                     '<kv9_engine::mem::MemEngine as kv9_engine::Engine>::snapshot',
                     '<kv9_engine::mem::MemEngine as kv9_engine::replicated::ReplicatedEngine>::write_applied')):
            address, size = int(address, 16), int(size, 16)
            lines = []
            for instruction in disassembly.splitlines():
                match = re.match(r'\s*([0-9a-f]+):', instruction)
                if match and address <= int(match[1], 16) < address + size:
                    lines.append(instruction)
            chosen.append(dict(name=name, address=address, size=size, instructions=lines))
    assert len(chosen) == 6
    passes = [p for p in chosen if p['name'].endswith('::measured_read_pass')]
    assert sorted(p['size'] for p in passes) == [1545, 2022]
    slots = []
    for p in passes:
        body = '\n'.join(p['instructions'])
        calls = re.findall(r'call\s+QWORD PTR \[rax\+(0x(?:18|20))\]', body)
        assert len(calls) == 2 and len(set(calls)) == 1
        slot = calls[0]
        slots.append(slot)
        p.update(interface='get' if slot == '0x18' else 'get_resident',
                 vtable_slot=slot, api_call_sites=2,
                 interpretation='One call site per timing mode; API selection is outside the loops.')
        assert body.count('Timespec::now') == 4
        assert '::answer' not in body
    assert sorted(slots) == ['0x18', '0x20']
    arms[arm] = dict(binary_sha256=sha(binary), functions=chosen)
result = dict(complete=True, measurement_authorized=True, diagnostic_only=True, arms=arms,
              inspector_sha256=sha(Path(__file__)),
              scope='Static release correspondence plus independently accepted preparation. The two generic read loops use actual ReadView slots, with timer mode selected outside the loop. Snapshot and write_applied retain production implementations. No executed-instruction, cache or full Rust-equivalence proof is claimed.')
assert json.loads((S / 'inputs-accepted.json').read_text())['complete']
with (S / 'codegen-review.json').open('x') as out:
    json.dump(result, out, indent=2)
print(json.dumps({'complete': True, 'measurement_authorized': True, 'functions_per_arm': 6}))
