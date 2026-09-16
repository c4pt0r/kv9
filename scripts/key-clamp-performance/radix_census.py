#!/usr/bin/env python3
"""Count an ideal compressed radix topology; no Rust index or timing prediction."""
from collections import Counter
import hashlib
import itertools
import json
from pathlib import Path
import sys

S=Path(sys.argv[1]).resolve();load=lambda p:json.loads(p.read_text())
accepted=load(S/'inputs-accepted.json');assert accepted['complete']
p=S/'runs/prepare-timing-baseline.json'
assert hashlib.sha256(p.read_bytes()).hexdigest()==accepted['prepared_sha256']['prepare-timing-baseline']
data=load(p)

# Tuple nodes are an offline topology model, not an implementation candidate.
def build(keys,depth=0):
    assert keys and len(set(keys))==len(keys)
    if len(keys)==1:return (b'',False,{},keys[0])
    first,last=keys[0],keys[-1];end=depth
    while end<min(len(first),len(last)) and first[end]==last[end]:end+=1
    terminal=len(first)==end;start=int(terminal);children={}
    for label,group in itertools.groupby(keys[start:],lambda key:key[end]):
        children[label]=build(list(group),end+1)
    assert children and (terminal or len(children)>=2)
    return (first[depth:end],terminal,children,None)

def lookup(node,key):
    depth=0;visits=0
    while True:
        visits+=1;prefix,terminal,children,leaf=node
        if leaf is not None:return leaf==key,visits
        if key[depth:depth+len(prefix)]!=prefix:return False,visits
        depth+=len(prefix)
        if depth==len(key):return terminal,visits
        if key[depth] not in children:return False,visits
        node=children[key[depth]];depth+=1

def inspect(node):
    pending=[(node,1)];branches=leaves=terminals=prefix_bytes=0;fanout=Counter();hits=Counter()
    while pending:
        (prefix,terminal,children,leaf),depth=pending.pop()
        if leaf is not None:leaves+=1;hits[depth]+=1;continue
        branches+=1;terminals+=int(terminal);prefix_bytes+=len(prefix);fanout[len(children)]+=1
        if terminal:hits[depth]+=1
        pending.extend((child,depth+1) for child in children.values())
    return dict(branches=branches,leaves=leaves,branch_terminal_keys=terminals,compressed_prefix_bytes=prefix_bytes,
                fanout_histogram=dict(sorted(fanout.items())),hit_node_visits_histogram=dict(sorted(hits.items())),
                mean_hit_node_visits=sum(k*v for k,v in hits.items())/sum(hits.values()),maximum_hit_node_visits=max(hits))

# Validate binary-prefix routing in this census model, not the future algorithm.
boundary=sorted(b''.join(parts) for n in range(4) for parts in itertools.product((b'\0',b'\1',b'\xff'),repeat=n))
root=build(boundary)
for key in boundary:assert lookup(root,key)[0]
for key in boundary:assert not lookup(root,key+b'\2')[0]
results=[]
for entry in data['read_inputs']:
    keys=[bytes.fromhex(x) for x in entry['keys_hex']];assert keys==sorted(set(keys))
    root=build(keys);observation=inspect(root)
    assert observation['leaves']+observation['branch_terminal_keys']==len(keys)
    actual=Counter()
    for key in keys:
        present,visits=lookup(root,key);assert present;actual[visits]+=1
    assert dict(sorted(actual.items()))==observation['hit_node_visits_histogram']
    misses=next(q for q in entry['queries'] if q['operation']=='get_miss');miss_visits=[]
    for key in misses['queries_hex']:
        present,visits=lookup(root,bytes.fromhex(key));assert not present;miss_visits.append(visits)
    results.append(dict(dataset=entry['dataset'],workload=entry['workload'],keys=len(keys),**observation,
                        checked_all_hits=True,checked_misses=len(miss_visits),mean_miss_node_visits=sum(miss_visits)/len(miss_visits)))
result=dict(complete=True,rows=results,binary_prefix_model_keys=len(boundary),input_sha256=hashlib.sha256(p.read_bytes()).hexdigest(),
            implementation_exists=False,elapsed_time_recorded=False,
            limits='Ideal static topology over these validated datasets only. No allocator layout, mutation/COW cost, cache behavior, Rust implementation, algorithm proof, latency or DB speedup is established.')
with (S/'radix-topology.json').open('x') as f:json.dump(result,f,indent=2)
print(json.dumps(result))
