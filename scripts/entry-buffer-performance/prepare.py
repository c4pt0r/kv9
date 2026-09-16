#!/usr/bin/env python3
"""Generate matched stage-one harnesses from the qualified engine-interface loops."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve();S.mkdir(exist_ok=False)
Q=Path('/mnt/data/kv9-work/entry-buffer-qualification-20260916-first')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
declared=json.loads((Q/'next-performance-plan.json').read_text())
assert json.loads((Q/'qualification-audit.json').read_text())['complete']
assert sha(Q/'candidate/crates/engine/src/entry_buffer.rs')==declared['candidate_entry_buffer_sha256']
assert sha(Q/'candidate/crates/engine/src/mem.rs')==declared['candidate_engine_sha256']
for p,h in json.loads((Q/'sources.json').read_text()).items():assert sha(Path(p))==h,p
shutil.copyfile(Q/'next-performance-plan.json',S/'declared-plan.json')
shutil.copyfile('/mnt/data/kv9-work/outlined-mutation-path-20260916-first/groups.bin',S/'groups.bin')
assert sha(S/'groups.bin')==declared['corpus']['sha256']
plan=dict(declared_plan_sha256=sha(S/'declared-plan.json'),execution_base=subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip(),
          qualified_root=str(Q),orders=declared['stage_one']['orders'],write_passes=12,write_excluded_passes=1,
          read_epochs=12,read_warm_passes=12,snapshot_passes=12,read_warmup_passes=8,snapshot_warmup_passes=8,
          query_count=512,query_seed=71,snapshot_queries_per_pass=512,
          corpus_sha256=declared['corpus']['sha256'],cpu=4,helper_cpus='6-15,22-31',
          process_timeout_seconds=180,process_output_max_bytes=67108864,minimum_free_bytes=8589934592,
          expected_prepare_processes=4,expected_counting_processes=16,expected_timing_processes=32,
          cases=[dict(operation='write_applied',dataset=d,workload=w,pinned=p)
                 for d in ('original','wide128') for w in ('overwrite','unique_insert') for p in (False,True)],
          gates=dict(original_mean_percent=-10,original_p99_percent=2,original_mean_improves_both_orders=True,
                     wide128_mean_percent=2,wide128_p99_percent=2),
          scopes=declared['stage_one']['scope'],stage_two=declared['stage_two'],
          allocation_scope='Retain per-window requests/live/peak and separate untimed final-index requested bytes; allocation builds emit no elapsed time')
assert plan['execution_base']=='e17d1b33fa4d76cb03a7457942f4a1bbba6beabb'
(S/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')

def replace(source,old,new,count=1):
    assert source.count(old)==count,(old,source.count(old),count)
    return source.replace(old,new)

def transform(source):
    source=replace(source,'fn corpus(bytes: &[u8], unique: bool)', 'fn corpus(bytes: &[u8], unique: bool, wide: bool)')
    source=replace(source,'            ordinal += 1;','            assert_eq!(key.len(), 27);\n            if wide { key.resize(128, 0); }\n            ordinal += 1;')
    source=replace(source,'struct Workload {\n',"struct Workload {\n    dataset: &'static str,\n")
    source=replace(source,'fn new(bytes: &[u8], unique: bool) -> Self {\n        let groups = corpus(bytes, unique);',
        'fn new(bytes: &[u8], unique: bool, dataset: &\'static str) -> Self {\n        assert!(dataset == "original" || dataset == "wide128");\n        let groups = corpus(bytes, unique, dataset == "wide128");')
    source=replace(source,'        Self {\n            name:', '        Self {\n            dataset,\n            name:')
    source=replace(source,'"dataset":"original24","workload":self.name','"dataset":self.dataset,"workload":self.name')
    source=replace(source,'"dataset":"original24","workload":work.name','"dataset":work.dataset,"workload":work.name')
    source=replace(source,'json!({"workload":work.name,"pinned":pinned', 'json!({"dataset":work.dataset,"workload":work.name,"pinned":pinned')
    source=replace(source,'"old_views":if pinned {106}else{0},"position_checks":106,','"old_views":if pinned {106}else{0},"position_checks":106,"refusal_checks":212,')
    source=replace(source,'        verify_other_cfs(current.as_ref());\n', '''        verify_other_cfs(current.as_ref());
        for refused in [i, i + 1] {
            let mut pending = WriteBatch::new();
            pending.put(ColumnFamily::Default, work.final_rows[0].0.clone(), b"refused".to_vec());
            pending.put(ColumnFamily::Lock, SENTINEL_KEY.to_vec(), b"refused".to_vec());
            assert!(engine.write_applied(pending, position(refused)).is_err());
            verify_position(&engine, i + 1);
            let after = engine.snapshot().unwrap();
            assert_eq!(rows(after.as_ref()), rows(current.as_ref()));
            verify_other_cfs(after.as_ref());
        }
''')
    source=replace(source,'    let works = [Workload::new(&bytes, false), Workload::new(&bytes, true)];',
        '    let works = [Workload::new(&bytes, false, "original"), Workload::new(&bytes, true, "original"),\n        Workload::new(&bytes, false, "wide128"), Workload::new(&bytes, true, "wide128")];')
    source=replace(source,'find(|w| spec["workload"] == w.name)', 'find(|w| spec["workload"] == w.name && spec["dataset"] == w.dataset)')
    # The previously qualified timed apply loop and every timer boundary are unchanged.
    return source

timing=(R/'scripts/engine-interface-experiment/src/main.rs').read_text()
counted=(R/'scripts/engine-interface-counting/src/main.rs').read_text()
for profile,source,origin in [('timing',timing,'engine-interface-experiment'),('counting',counted,'engine-interface-counting')]:
    work=S/('harness-'+profile);(work/'src').mkdir(parents=True)
    result=transform(source)
    if profile=='counting':
        extra='''fn index_footprint(work: &Workload) -> Value {
    let initial = counting::live();
    let engine = populated_engine(work);
    let retained = counting::live() - initial;
    drop(engine);
    assert_eq!(counting::live(), initial);
    json!({"requested_bytes":retained,"default_keys":work.final_rows.len(),"sentinel_keys":2,
        "fully_reclaimed":true,"includes_stack_bytes":false,"elapsed_time_recorded":false})
}

'''
        result=replace(result,'fn measure_apply(work: &Workload, pinned: bool) -> Vec<Value> {',extra+'fn measure_apply(work: &Workload, pinned: bool) -> Vec<Value> {')
        result=replace(result,'"final_keys":work.final_rows.len(),"final_state_sha256":digest(&work.final_rows),',
            '"final_keys":work.final_rows.len(),"index_footprint":index_footprint(work),"final_state_sha256":digest(&work.final_rows),')
        shutil.copyfile(R/'scripts/engine-interface-counting/src/counting.rs',work/'src/counting.rs')
    (work/'src/main.rs').write_text(result)
    shutil.copyfile(R/f'scripts/{origin}/src/probes.rs',work/'src/probes.rs')
    shutil.copyfile(R/f'scripts/{origin}/Cargo.lock',work/'Cargo.lock')
    manifest=(R/f'scripts/{origin}/Cargo.toml').read_text().replace('kv9-'+origin,'kv9-entry-buffer-'+profile)
    (work/'Cargo.toml').write_text(manifest)
    # Ensure the write loop remains byte-for-byte the inherited timed/counted loop.
    old=source[source.index('fn apply_pass('):source.index('fn measure_apply(')]
    new=result[result.index('fn apply_pass('):result.index('fn index_footprint(') if profile=='counting' else result.index('fn measure_apply(')]
    assert old==new
correspondence=dict(complete=True,operation_transform='Same shared dataset/refusal transformation in both inherited harnesses; unchanged write loops and windows. Count-only final footprint observation is outside all windows.',
                   origins={str(R/f'scripts/{d}/src/main.rs'):sha(R/f'scripts/{d}/src/main.rs') for d in ('engine-interface-experiment','engine-interface-counting')},
                   generated={str(p):sha(p) for profile in ('timing','counting') for p in (S/('harness-'+profile)/'src').glob('*.rs')})
(S/'harness-correspondence.json').write_text(json.dumps(correspondence,indent=2)+'\n')
print(json.dumps({'complete':True,'root':str(S),'cases':len(plan['cases'])}))
