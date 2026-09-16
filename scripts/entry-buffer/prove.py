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
P=R/'proofs/lean/entry-buffer/EntryBuffer.lean'
C=R/'proofs/lean/entry-buffer/source-contract.json'
contract=json.loads(C.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def require(condition,message):
    if not condition:raise ValueError(message)
def bind(data,digest):require(hashlib.sha256(data).hexdigest()==digest,'reviewed source hash mismatch')
for path,digest in contract['sources'].items():
    root,relative=path.split('/',1);bind(((R if root=='repo' else S)/relative).read_bytes(),digest)
upstream=contract['rpds_source'];bind(Path(upstream['path']).read_bytes(),upstream['sha256'])
proof=P.read_text();names=re.findall(r'^theorem (\w+)',proof,re.M)
require(names==contract['theorems'],'theorem inventory mismatch')
require(sha(lean)==contract['lean_sha256'],'Lean binary mismatch')
out=S/'proof';out.mkdir(exist_ok=False)
result=dict(complete=False,accepted=False,started_ns=time.time_ns(),controls=[],sources=contract['sources'],
            lean=dict(path=str(lean),sha256=sha(lean)),scope=contract['scope'])
def check(source,name):
    work=out/name;work.mkdir()
    queries='\n'.join('#print axioms Kv9.EntryBuffer.'+n for n in names)
    (work/'EntryBuffer.lean').write_text(source+'\n'+queries+'\n')
    with (work/'lean.log').open('x') as stdout:
        p=subprocess.run([str(lean),'-DwarningAsError=true','-o','EntryBuffer.olean','EntryBuffer.lean'],cwd=work,stdout=stdout,stderr=subprocess.STDOUT,timeout=60)
    log=(work/'lean.log').read_text()
    require(p.returncode==0,'Lean rejected proof')
    axioms={}
    for name in names:
        qualified='Kv9.EntryBuffer.'+name
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
        ('wrong-key-boundary','entry.bytes.take entry.keyLength','entry.bytes.take (entry.keyLength + 1)','Lean rejected proof'),
        ('wrong-value-boundary','entry.bytes.drop entry.keyLength','entry.bytes.drop (entry.keyLength + 1)','Lean rejected proof'),
        ('whole-buffer-order','compare (key a) (key b)','compare a.bytes b.bytes','Lean rejected proof'),
        ('whole-buffer-equality','key a == key b','a.bytes == b.bytes','Lean rejected proof'),
        ('omit-cap-check','if k + v ≤ cap then some (k + v) else none else none','some (k + v) else none','Lean rejected proof'),
        ('replace-with-empty-value','encode k v :: storedErase k state','encode k [] :: storedErase k state','Lean rejected proof'),
        ('retain-deleted-key','!(query == borrow entry)','(query == borrow entry)','Lean rejected proof'),
        ('reverse-history','history.foldl storedApply state','history.reverse.foldl storedApply state','Lean rejected proof'),
        ('mutate-rejected-batch','else (false, state)','else (false, history.foldl storedApply state)','Lean rejected proof'),
        ('proof-hole','  simp [key, encode]','  sorry','Lean rejected proof'),
        ('custom-axiom','  simp [key, encode]','  exact False.elim fake','untrusted axiom dependency'),
    ]:
        require(old in proof,'control anchor missing: '+label)
        changed=proof.replace(old,new)
        if label=='custom-axiom':changed=changed.replace('namespace Kv9.EntryBuffer','namespace Kv9.EntryBuffer\naxiom fake : False',1)
        try:check(changed,'control-'+label)
        except ValueError as error:
            require(reason in str(error),'control rejected at wrong gate: '+label)
            result['controls'].append(dict(name=label,rejected=True,reason=str(error)))
        else:raise ValueError('invalid proof accepted: '+label)
    source=(R/'scripts/entry-buffer/entry_buffer.rs').read_bytes()
    for name,old,new in [
        ('remove-cap-check',b'*total <= isize::MAX as usize',b'*total <= usize::MAX'),
        ('change-byte-order',b'self.key().cmp(other.key())',b'self.bytes.cmp(&other.bytes)')]:
        require(source.count(old)==1,'source control anchor mismatch')
        changed=source.replace(old,new)
        (out/(name+'.rs')).write_bytes(changed)
        try:bind(changed,contract['sources']['repo/scripts/entry-buffer/entry_buffer.rs'])
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
