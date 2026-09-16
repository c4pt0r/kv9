#!/usr/bin/env python3
"""Create matched engine/common workspaces with one private representation change."""
import difflib
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

R = Path(__file__).resolve().parents[2]
S = Path(sys.argv[1]).resolve()
S.mkdir(exist_ok=False)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=R, text=True).strip()
assert head == '14f35ecb772e060b58b6898e648630a72033bf8c'
plan = dict(status='Qualification only; no timing or production promotion', source_base=head,
            scope='Private safe inline key <= 40 bytes, Vec fallback; original dependencies',
            gates=['byte identity and explicit byte ordering', 'actual layout and allocation requests',
                   'independent model and existing engine/common correctness', 'conditional Lean proof with rejecting controls'],
            next='Predeclare allocation/timing only after qualification, including long-key cost',
            preserved=['Raft/WAL/read authorization', 'public/wire/storage types', 'mutation and refusal ordering'],
            output_policy='/mnt/data/kv9-work; reuse repository target; local CI only')
(S / 'qualification-plan.json').write_text(json.dumps(plan, indent=2)+'\n')
root_manifest = (R / 'Cargo.toml').read_text().split('\n[package]\n')[0]
start = root_manifest.index('members = ['); end = root_manifest.index('\n]', start)+2
root_manifest = root_manifest[:start]+'members = ["crates/common", "crates/engine"]'+root_manifest[end:]
for arm in ('baseline', 'candidate'):
    work = S / arm
    work.mkdir()
    (work / 'Cargo.toml').write_text(root_manifest)
    shutil.copyfile(R / 'Cargo.lock', work / 'Cargo.lock')
    for name in ('common', 'engine'):
        shutil.copytree(R / 'crates' / name, work / 'crates' / name)
    shutil.copyfile(R / 'scripts/inline-key/model.rs', work / 'crates/engine/tests/inline_key_model.rs')
    if arm == 'candidate':
        p = work / 'crates/engine/src/mem.rs'
        original = p.read_text()
        source, tests = original.split('#[cfg(test)]', 1)
        replacements = [
            ('use std::sync::RwLock;', 'use std::sync::RwLock;\nuse crate::mem_key::MapKey;', 1),
            ('RedBlackTreeMapSync<Vec<u8>, Vec<u8>>', 'RedBlackTreeMapSync<MapKey, Vec<u8>>', 1),
            ('start.to_vec()..end.to_vec()', 'MapKey::from_slice(start)..MapKey::from_slice(end)', 5),
            ('..=target.to_vec()', '..=MapKey::from_slice(target)', 1),
            ('(k.clone(), v.clone())', '(k.as_slice().to_vec(), v.clone())', 5),
            ('insert_mut(key.clone(), value.clone())', 'insert_mut(MapKey::from_slice(key), value.clone())', 2),
            ('remove_mut(key)', 'remove_mut(key.as_slice())', 2),
            ('let doomed: Vec<Vec<u8>>', 'let doomed: Vec<MapKey>', 1),
            ('for b in k.iter().chain(v.iter())', 'for b in k.as_slice().iter().chain(v.iter())', 1),
        ]
        for old, new, count in replacements:
            assert source.count(old) == count, (old, source.count(old), count)
            source = source.replace(old, new)
        p.write_text(source+'#[cfg(test)]'+tests)
        (S / 'mem.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True), p.read_text().splitlines(True), fromfile='baseline/mem.rs', tofile='candidate/mem.rs')))
        lib = work / 'crates/engine/src/lib.rs'
        lib.write_text(lib.read_text().replace('pub mod mem;', 'pub mod mem;\nmod mem_key;'))
        shutil.copyfile(R / 'scripts/inline-key/mem_key.rs', work / 'crates/engine/src/mem_key.rs')
    # Prune unrelated root-package entries once, before --locked builds.
    with (S / (arm+'-metadata.json')).open('x') as out, (S / (arm+'-metadata.stderr')).open('x') as err:
        subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=work,stdout=out,stderr=err,check=True,timeout=120)
sources = {str(p):sha(p) for arm in ('baseline','candidate') for p in sorted((S/arm).rglob('*')) if p.is_file()}
sources.update({str(p):sha(p) for p in (R/'scripts/inline-key').glob('*.rs')})
sources[str(Path(__file__).resolve())] = sha(Path(__file__).resolve())
(S/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
print(json.dumps({'complete':True,'source_files':len(sources),'root':str(S)}))
