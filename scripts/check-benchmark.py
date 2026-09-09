#!/usr/bin/env python3
"""Independently verify raw benchmark trials and their complete matrix summary."""
import argparse
import json
from pathlib import Path
from benchmark_report import validate_matrix

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('root',type=Path)
p.add_argument('--build',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--expected-revision')
a=p.parse_args()
result=validate_matrix(a.root,a.build,a.expected_revision)
with a.output.open('x') as f: json.dump(result,f,indent=2);f.write('\n')
print(f"PASS: {len(result['trials'])} repeated trials and {len(result['guards'])} complete-history guards independently verified")
