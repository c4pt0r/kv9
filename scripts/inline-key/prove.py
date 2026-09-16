#!/usr/bin/env python3
"""Check representation refinement and reject semantic/source/proof-hole controls."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve();lean=Path(sys.argv[2]).resolve()
P=R/'proofs/lean/inline-key/InlineKey.lean'
C=R/'proofs/lean/inline-key/source-contract.json'
contract=json.loads(C.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def require(condition,message):
    if not condition:raise ValueError(message)
def bind(data,digest):require(hashlib.sha256(data).hexdigest()==digest,'reviewed source hash mismatch')
for path,digest in contract['sources'].items():
    root,relative=path.split('/',1);bind(((R if root=='repo' else S)/relative).read_bytes(),digest)
proof=P.read_text();names=re.findall(r'^theorem (\w+)',proof,re.M)
require(names==contract['theorems'],'theorem inventory mismatch')
require(sha(lean)==contract['lean_sha256'],'Lean binary mismatch')
out=S/'proof';out.mkdir(exist_ok=False)
result=dict(complete=False,accepted=False,started_ns=time.time_ns(),controls=[],sources=contract['sources'],
            lean=dict(path=str(lean),sha256=sha(lean)),scope=contract['scope'])
def check(source,name):
    work=out/name;work.mkdir()
    queries='\n'.join('#print axioms Kv9.InlineKey.'+n for n in names)
    (work/'InlineKey.lean').write_text(source+'\n'+queries+'\n')
    with (work/'lean.log').open('x') as stdout:
        p=subprocess.run([str(lean),'-DwarningAsError=true','-o','InlineKey.olean','InlineKey.lean'],cwd=work,stdout=stdout,stderr=subprocess.STDOUT,timeout=60)
    log=(work/'lean.log').read_text()
    require(p.returncode==0,'Lean rejected proof')
    axioms={}
    for name in names:
        qualified='Kv9.InlineKey.'+name
        matches=list(re.finditer(re.escape("'"+qualified+"'")+r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)',log))
        require(len(matches)==1,'axiom inventory missing')
        used={x.strip() for x in (matches[0][1] or '').split(',') if x.strip()}
        require(used <= {'propext','Classical.choice','Quot.sound'},'untrusted axiom dependency')
        axioms[qualified]=sorted(used)
    (work/'axioms.json').write_text(json.dumps(axioms,indent=2)+'\n')
    return axioms
try:
    result['theorems']=check(proof,'positive')
    for label,old,new,reason in [
        ('expose-padding','storage.take len','storage','Lean rejected proof'),
        ('length-only-order','compare (decode a) (decode b)','compare (decode a).length (decode b).length','Lean rejected proof'),
        ('length-only-equality','decode a == decode b','(decode a).length == (decode b).length','Lean rejected proof'),
        ('retain-deleted-key','!(query == borrowed entry.1)','(query == borrowed entry.1)','Lean rejected proof'),
        ('reverse-history','history.foldl storedApply state','history.reverse.foldl storedApply state','Lean rejected proof'),
        ('proof-hole','  unfold encode\n  split <;> simp [decode]','  sorry','Lean rejected proof'),
        ('custom-axiom','  unfold encode\n  split <;> simp [decode]','  exact False.elim fake','untrusted axiom dependency'),
    ]:
        require(old in proof,'control anchor missing: '+label)
        changed=proof.replace(old,new)
        if label=='custom-axiom':changed=changed.replace('namespace Kv9.InlineKey','namespace Kv9.InlineKey\naxiom fake : False',1)
        try:check(changed,'control-'+label)
        except ValueError as error:
            require(reason in str(error),'control rejected at wrong gate: '+label)
            result['controls'].append(dict(name=label,rejected=True,reason=str(error)))
        else:raise ValueError('invalid proof accepted: '+label)
    source=(R/'scripts/inline-key/mem_key.rs').read_bytes()
    for name,old,new in [('change-capacity',b'usize = 40',b'usize = 41'),
                         ('change-byte-order',b'self.as_slice().cmp(other.as_slice())',b'self.as_slice().len().cmp(&other.as_slice().len())')]:
        require(source.count(old)==1,'source control anchor mismatch')
        changed=source.replace(old,new)
        (out/(name+'.rs')).write_bytes(changed)
        try:bind(changed,contract['sources']['repo/scripts/inline-key/mem_key.rs'])
        except ValueError as error:result['controls'].append(dict(name=name,rejected=True,reason=str(error)))
        else:raise ValueError('invalid source accepted')
    for path,digest in contract['sources'].items():
        root,relative=path.split('/',1);bind(((R if root=='repo' else S)/relative).read_bytes(),digest)
    result.update(complete=True,accepted=True)
except BaseException as error:
    result['failure']=repr(error);raise
finally:
    result['ended_ns']=time.time_ns();(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'accepted':True,'theorems':len(result['theorems']),'rejecting_controls':len(result['controls'])}))
