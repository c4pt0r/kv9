#!/usr/bin/env python3
"""Retain comparator excerpts from the exact three measured timing executables."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

S=Path(sys.argv[1]).resolve();build=json.loads((S/'build-summary.json').read_text());assert build['complete']
out=S/'codegen';out.mkdir(exist_ok=False);records={}
for arm in ('baseline','single_buffer_control','clamped_candidate'):
    binary=build['arms'][arm]['timing'];assert hashlib.sha256(Path(binary['path']).read_bytes()).hexdigest()==binary['sha256']
    text=subprocess.check_output(['objdump','-Cd',binary['path']],text=True,timeout=30)
    a=text.index('<rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins>:');function=text[a:text.index('\n\n',a)]
    (out/(arm+'-insert.txt')).write_text(function+'\n')
    prefix=function[:function.index('<memcmp@GLIBC_2.2.5>')]
    branches=re.findall(r'\tja\s+([a-f0-9]+)',prefix)
    assert len(branches)==(2 if arm=='single_buffer_control' else 0)
    records[arm]=dict(elf_sha256=binary['sha256'],unsigned_bound_failure_branches=branches,
                      insertion_excerpt_sha256=hashlib.sha256((function+'\n').encode()).hexdigest())
result=dict(complete=True,arms=records,limits='Static codegen identity only; elapsed effects come from the separate matched workload screen.')
(out/'review.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'complete':True,'bounds_branches':{a:len(r['unsigned_bound_failure_branches']) for a,r in records.items()}}))
