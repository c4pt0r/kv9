#!/usr/bin/env python3
"""Prepare an isolated engine key/value-buffer candidate with original dependencies."""
import difflib
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve();S.mkdir(exist_ok=False)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip()
assert head=='e9ebdc42f2171fd5ab62f3fc2d2d010d88e55d22'
shutil.copyfile(R/'docs/inline-key-performance-v1/next-entry-buffer-plan.json',S/'declared-plan.json')
plan=dict(status='Source/layout/model/proof qualification; no timing or production promotion',source_base=head,
          representation='Private EntryBuffer { Box<[u8]>, key_len }; CfMap<EntryBuffer, ()>',
          length_contract='Checked sum <= isize::MAX before allocating. Validate every Put before changing any CF; replicated-position refusal retains priority. Allocation failure keeps ordinary Vec/Box behavior.',
          preserved=['Public key/value/wire/storage types','Ordered mutations, snapshot ownership and position/revision publication','Raft/WAL/read authorization'],
          next='Declare performance only after source/model/proof/layout qualification',output_policy='/mnt/data/kv9-work with reused repository target; local CI only')
(S/'qualification-plan.json').write_text(json.dumps(plan,indent=2)+'\n')
manifest=(R/'Cargo.toml').read_text().split('\n[package]\n')[0]
a=manifest.index('members = [');b=manifest.index('\n]',a)+2
manifest=manifest[:a]+'members = ["crates/common", "crates/engine"]'+manifest[b:]
for arm in ('baseline','candidate'):
    work=S/arm;work.mkdir();(work/'Cargo.toml').write_text(manifest);shutil.copyfile(R/'Cargo.lock',work/'Cargo.lock')
    for name in ('engine','common'):shutil.copytree(R/'crates'/name,work/'crates'/name)
    shutil.copyfile(R/'scripts/inline-key/model.rs',work/'crates/engine/tests/inline_key_model.rs')
    shutil.copyfile(R/'scripts/entry-buffer/model.rs',work/'crates/engine/tests/entry_buffer_model.rs')
    if arm=='candidate':
        p=work/'crates/engine/src/mem.rs';original=p.read_text();source,tests=original.split('#[cfg(test)]',1)
        changes=[
          ('use std::sync::RwLock;','use std::sync::RwLock;\nuse crate::entry_buffer::EntryBuffer;',1),
          ('RedBlackTreeMapSync<Vec<u8>, Vec<u8>>','RedBlackTreeMapSync<EntryBuffer, ()>',1),
          ('self.cf(cf).get(key).cloned()','self.cf(cf).get_key_value(key).map(|(entry, _)| entry.value().to_vec())',1),
          ('start.to_vec()..end.to_vec()','EntryBuffer::from_slices(start, &[])..EntryBuffer::from_slices(end, &[])',5),
          ('..=target.to_vec()','..=EntryBuffer::from_slices(target, &[])',1),
          ('.map(|(k, v)| (k.clone(), v.clone()))','.map(|(entry, _)| (entry.key().to_vec(), entry.value().to_vec()))',2),
          ('.map(|(k, v)| Ok((k.clone(), v.clone())))','.map(|(entry, _)| Ok((entry.key().to_vec(), entry.value().to_vec())))',3),
          ('insert_mut(key.clone(), value.clone())','insert_mut(EntryBuffer::from_slices(key, value), ())',2),
          ('remove_mut(key)','remove_mut(key.as_slice())',2),
          ('.map(|(k, _)| k.clone())','.map(|(entry, _)| entry.key().to_vec())',1),
          ('map.remove_mut(&k)','map.remove_mut(k.as_slice())',1),
          ('for (k, v) in state.cf(cf).range','for (entry, _) in state.cf(cf).range',1),
          ('for b in k.iter().chain(v.iter())','for b in entry.key().iter().chain(entry.value().iter())',1),
          ('Some(self.state.cf(cf).get(key).map(Vec::as_slice))','Some(self.state.cf(cf).get_key_value(key).map(|(entry, _)| entry.value()))',1),
          ('        for m in batch.mutations() {','        validate_entry_lengths(&batch)?;\n        for m in batch.mutations() {',2),
        ]
        for old,new,count in changes:
            assert source.count(old)==count,(old,source.count(old),count);source=source.replace(old,new)
        helper='''// This preflight precedes all mutations. It adds no public key/value limit:
// it refuses only a sum that one Rust allocation cannot represent. In the
// replicated path the original position refusal is checked before this helper.
fn validate_entry_lengths(batch: &WriteBatch) -> Result<()> {
    if batch.mutations().iter().any(|mutation| match mutation {
        Mutation::Put { key, value, .. } => EntryBuffer::checked_len(key.len(), value.len()).is_none(),
        Mutation::Delete { .. } => false,
    }) {
        return Err(Error::Engine("engine: combined key/value length exceeds an addressable buffer".into()));
    }
    Ok(())
}

'''
        source=source.replace('/// All column families as one value.',helper+'/// All column families as one value.',1)
        p.write_text(source+'#[cfg(test)]'+tests)
        subprocess.run(['rustfmt','--edition','2021','--config','skip_children=true',str(p)],check=True,timeout=30)
        (S/'mem.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True),p.read_text().splitlines(True),fromfile='baseline/mem.rs',tofile='candidate/mem.rs')))
        lib=work/'crates/engine/src/lib.rs';lib.write_text(lib.read_text().replace('pub mod mem;','pub mod mem;\nmod entry_buffer;'))
        shutil.copyfile(R/'scripts/entry-buffer/entry_buffer.rs',work/'crates/engine/src/entry_buffer.rs')
    with (S/(arm+'-metadata.json')).open('x') as out,(S/(arm+'-metadata.stderr')).open('x') as err:
        subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=work,stdout=out,stderr=err,check=True,timeout=120)
sources={str(p):sha(p) for arm in ('baseline','candidate') for p in sorted((S/arm).rglob('*')) if p.is_file()}
for p in [Path(__file__).resolve(),R/'scripts/inline-key/model.rs',R/'scripts/inline-key/test.py',*sorted((R/'scripts/entry-buffer').glob('*.rs'))]:sources[str(p)]=sha(p)
(S/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
print(json.dumps({'complete':True,'root':str(S),'source_files':len(sources)}))
