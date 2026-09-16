#!/usr/bin/env python3
"""Erase only declared instrumentation; require identical remaining source bytes."""
import hashlib
import json
from pathlib import Path
import sys

S=Path(sys.argv[1]).resolve()
t=(S/'harness-timing/src/main.rs').read_text();c=(S/'harness-counting/src/main.rs').read_text()
c=c.replace(c.splitlines()[0],t.splitlines()[0],1)
c=c.replace('mod counting;\n','use std::time::Instant;\n')
c=c.replace('static ALLOCATOR: counting::Allocator = counting::Allocator;','static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;')
a=c.index('// All buffers are reserved before the counted operation windows.');b=c.index('fn apply_pass(',a)
x=t.index('#[derive(Default)]\nstruct Metrics');y=t.index('fn apply_pass(',x)
c=c[:a]+t[x:y]+c[b:]
a=c.index('fn index_footprint(');b=c.index('fn measure_apply(',a);c=c[:a]+c[b:]
c=c.replace('"index_footprint":index_footprint(work),','')
assert c.count('let started = begin();')==4
c=c.replace('let started = begin();','let started = Instant::now();')
c=c.replace('    let snapshot_bytes = std::mem::size_of_val(engine.snapshot().unwrap().as_ref());\n','').replace('"snapshot_bytes":snapshot_bytes,','')
c=c.replace('    result["counting_build"] = json!(true);\n','')
c=c.replace('let mut measured = match spec["operation"].as_str().unwrap() {','let measured = match spec["operation"].as_str().unwrap() {')
block='''        for row in &mut measured {
            let scope = std::mem::replace(&mut row["unit"], json!("allocator_counts_per_window"));
            row["operation_window"] = scope;
        }
'''
assert c.count(block)==1;c=c.replace(block,'')
assert c==t,'normalized counting source differs from timing'
result=dict(complete=True,timing_source_sha256=hashlib.sha256(t.encode()).hexdigest(),normalized_counting_sha256=hashlib.sha256(c.encode()).hexdigest(),scope='All source bytes match after erasing inherited instrumentation/metadata and explicitly untimed snapshot-size/final-index-footprint observations; all operation bodies, lifetimes and refusal checks match.')
p=S/'harness-audit.json'
if p.exists():assert json.loads(p.read_text())==result
else:p.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
