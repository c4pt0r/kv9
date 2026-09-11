#!/usr/bin/env python3
import collections,gzip,hashlib,json,os,pathlib,time
if not __debug__:raise RuntimeError('assertions required')
ROOT=pathlib.Path(__file__).parent
P=pathlib.Path('/tmp/kv9-crc-profile-active-prefix-preparation')
S=pathlib.Path('/tmp/kv9-crc-profile-active-prefix-supervision-first')
OLD=pathlib.Path('/tmp/kv9-crc-write-profile-first')
DIAG=pathlib.Path('/tmp/kv9-crc-profile-coverage-investigation')
SERVER=pathlib.Path('/tmp/kv9-wal-crc32-release-first')
SOURCE=pathlib.Path('/tmp/kv9-wal-crc32-table')
CLIENT=pathlib.Path('/tmp/kv9-point-write-v3-release-first/native')
def read(p):return json.loads(p.read_text())
def sha_bytes(b):return hashlib.sha256(b).hexdigest()
def sha(p):return sha_bytes(p.read_bytes())
def save(p,v):
 with p.open('x')as f:json.dump(v,f,indent=2,sort_keys=True);f.write('\n')
assert sorted(os.sched_getaffinity(0))==list(range(6,16))+list(range(22,32))
pre=read(S/'preflight-first.json')
assert all(sha(pathlib.Path(k))==v for k,v in pre['source_bindings'].items())
assert sha(SERVER/'kv9')=='b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13'
assert (P/'analyze.py').read_bytes()==(OLD/'analyze.py').read_bytes()
index={}
def copy(q,relative):
 q=pathlib.Path(q);assert q.is_file() and not q.is_symlink(),q
 raw=q.read_bytes();encoding='identity'
 if len(raw)>131072:payload=gzip.compress(raw,compresslevel=6,mtime=0);relative+='.gz';encoding='gzip'
 else:payload=raw
 dst=ROOT/relative;dst.parent.mkdir(parents=True,exist_ok=True)
 with dst.open('xb')as f:f.write(payload)
 back=dst.read_bytes();assert (gzip.decompress(back)if encoding=='gzip'else back)==raw
 assert q.read_bytes()==raw
 index[str(q)]={'stored_path':relative,'encoding':encoding,'original_bytes':len(raw),'original_sha256':sha_bytes(raw),'stored_bytes':len(back),'stored_sha256':sha_bytes(back)}
for n in ['profile.py','active_prefix.py','analyze.py','test_prefix_contract.py','qualify_cleanup.py','PREPARATION.json','inventory.json','README.md','profile.diff','protocol.preview.json','READINESS-after-cleanup.json','cleanup-qualification-preparation.json','CLEANUP-QUALIFICATION.md','offline-contract-first.log','offline-contract-tool-receipt-first.json','offline-contract-invocation-first.json','offline-contract-terminal-first.json','offline-contract-source-before-first.json','offline-contract-source-after-first.json','cleanup-qualification-first.log','cleanup-tool-receipt-first.json','cleanup-invocation-first.json','cleanup-terminal-first.json','cleanup-source-before-first.json','cleanup-source-after-first.json']:
 copy(P/n,'qualified/'+n)
for n in ['cleanup-qualification-first/summary.json','cleanup-qualification-first/post_spawn_exception/active-prefix/summary.json','cleanup-qualification-first/timeout/active-prefix/summary.json']:
 copy(P/n,'qualified/'+n)
for q in sorted(S.iterdir()):
 if q.is_file():copy(q,'supervision/'+q.name)
for n in ['summary.json','analysis-summary.json','protocol.json','build-bindings.json','preflight.json','RESULT-first.json','READOUT-first.md']:copy(P/n,'runtime/'+n)
per_files=['profile-plan.json','requested-config.json','profile-record.json','profile-summary.json','measured-samples.json','perf-command.json','perf-record.log','permission.stdout','permission.stderr','decode-commands.json','perf-script.txt','perf-script.stderr','perf-top.txt','perf-top.stderr','run/report.json','run/ready.json','fixture-result.json','client-identity.json','client-exit.json','cleanup.json','resource-coverage.json','resource-samples.json','listener-before.json','listener-after.json','sample-calls.json','storage-mounts.json','storage-resource-samples.json','fixture/tmpfs-retention.json','active-prefix/summary.json']
profiles=[]
for api in ['point_put','batch_put']:
 d=P/api
 for n in per_files:copy(d/n,'runtime/'+api+'/'+n)
 for q in sorted(d.glob('*fresh-drain.json')):copy(q,'runtime/'+api+'/'+q.name)
 for q in sorted(d.glob('readback-*.txt')):copy(q,'runtime/'+api+'/'+q.name)
 samples=read(d/'measured-samples.json')
 offsets=collections.Counter(x['frames'][0]['offset']for x in samples if 'write_record_unsynced'in x['frames'][0]['symbol'])
 loop=sum(v for k,v in offsets.items()if k is not None and (0x130<=k<0x1a9 or 0x1b0<=k<0x1c6))
 profiles.append({'api':api,'selected_samples':len(samples),'method_leaf_samples':sum(offsets.values()),'fnv_loop_site_leaf_samples':loop,'fnv_loop_site_percent_of_selected':100*loop/len(samples),'other_method_leaf_samples':sum(offsets.values())-loop,'method_leaf_offsets':{str(k):v for k,v in sorted(offsets.items())},'input_selected_samples_sha256':sha(d/'measured-samples.json')})
for n in ['REPORT.md','diagnosis.json','RESULT.json','original-runtime-terminal-first.json','original-decode-invocation-first.json','original-decode-terminal-first.json','original-decode-first.log']:
 copy(DIAG/n,'previous-rejection/'+n)
for n in ['summary.json','protocol.json','runtime-invocation-first.json','runtime-terminal-first.json','decode-invocation-first.json','decode-terminal-first.json','decode-first.log','batch_put/profile-record.json','batch_put/run/report.json','batch_put/requested-config.json','batch_put/decode-commands.json']:
 copy(OLD/n,'previous-rejection/original/'+n)
for n in ['crates/raft/src/storage.rs','crates/engine/src/wal.rs','crates/engine/src/wal_segment.rs']:
 assert sha(SOURCE/n)==read(SERVER/'build.json')['sources'][n]
 copy(SOURCE/n,'source/'+n)
for n in ['build.json','kv9-cargo.jsonl']:copy(SERVER/n,'build/server/'+n)
for n in ['build.json','sources.json']:copy(CLIENT/n,'build/client/'+n)
binary_sha=sha(SERVER/'kv9')
assert binary_sha==pre['source_bindings'][str(SERVER/'kv9')]
fields=(ROOT/'write-record-symbol-first.txt').read_text().strip().split();assert fields[:2]==['0000000000509c50','0000000000000404']
asm=(ROOT/'write-record-disassembly-first.txt').read_text()
assert '509d80:'in asm and '509df7:'in asm and '509e00:'in asm and '509e14:'in asm and '$0x1000193'in asm and '$0x811c9dc5'in asm
attribution={'scope':'Read-only mapping of already-selected sampled leaf instruction offsets to this exact binary, not new profiling or decoder execution.','binary_path':str(SERVER/'kv9'),'binary_sha256':binary_sha,'source_path':str(SOURCE/'crates/raft/src/storage.rs'),'source_sha256':sha(SOURCE/'crates/raft/src/storage.rs'),'source_fnv1a_lines':[60,66],'source_call_line':363,'symbol':fields[3],'elf_symbol_address':fields[0],'symbol_bytes':int(fields[1],16),'hash_loop_offset_intervals_half_open':[[0x130,0x1a9],[0x1b0,0x1c6]],'commands':[{'argv':['nm','-S','--defined-only',str(SERVER/'kv9')],'exit_code':0,'retained_output':'write-record-symbol-first.txt','output_scope':'one matching symbol line selected from full nm stdout'},{'argv':['objdump','-d','--start-address=0x509c50','--stop-address=0x50a054',str(SERVER/'kv9')],'exit_code':0,'retained_output':'write-record-disassembly-first.txt'}],'terminal_tool_chunk':'54c29d','profiles':profiles,'limits':['Source and exact instructions identify inlined FNV-1a byte-load/XOR/multiply loops and their loop control.','No separate fnv1a frame is required or present in the retained representative leaf stacks.','A sampled instruction pointer is statistical on-CPU attribution, subject to sampling skid; it does not measure every invocation or all inclusive callee CPU.','All 283 BatchPut method leaves were at these loop sites in this recording; that is not a claim that all write_record_unsynced function CPU is checksum work.','FNV-1a in Raft storage is distinct from IEEE CRC32 frame_crc in engine storage.']}
save(ROOT/'write-record-attribution-first.json',attribution)
for path,row in index.items():
 assert sha(pathlib.Path(path))==row['original_sha256'];raw=(ROOT/row['stored_path']).read_bytes();assert sha_bytes(raw)==row['stored_sha256'];assert sha_bytes(gzip.decompress(raw)if row['encoding']=='gzip'else raw)==row['original_sha256']
save(ROOT/'original-path-index.json',index)
readme='''# CRC active-prefix CPU diagnostic publication bundle

The runtime and unchanged decoder each passed once: sessions 76156/21261, both exit 0. Exact-byte originals are stored under qualified/, supervision/, runtime/, source/, build/ and previous-rejection/. original-path-index.json maps each original absolute path to its retained file, original hash/size and stored hash/size. Files over 128 KiB use deterministic gzip; decompressing restores the exact original bytes. Every copy and decompression was read back and checked against the unchanged original.

Point PUT selected 3,286 samples; BatchPut64 selected 3,103. Both passed strict nominal prefix/tail containment, one-millisecond edge exclusion, clock-spread bounds, 32-bin coverage, zero-loss, raw cap and original report/retention gates. Each separately retained active prefix contains 128 successful unary absent-key reads and zero retries; all 256 CLI lifetimes, eight fixture lifetimes and four recorded profiler lifetimes exited. Instrumented measured calls were 582,235 and 63,324, all successful, and are not QPS acceptance.

Priming changes cache, metadata, allocation and CPU state before the unchanged five-second measurement. These are one-run CPU attribution diagnostics on a shared host with volatile tmpfs WAL, not repeated A/B timing, full linearizability or equal-durability evidence. Inclusive recovered stacks overlap and are not additive; leaf sample fractions are not end-to-end latency fractions. Original prepared README/status files remain historical snapshots; later qualification and runtime receipts establish completed work without rewriting them.

The prior post-CRC attempt remains rejected for missing BatchPut prefix coverage. Its original runtime/decoder terminals, rejected interval report/record, and retained passive diagnosis are included in previous-rejection/. No start-time trimming, tail extension, decoder weakening or failure replacement occurred.

write-record-attribution-first.json and exact symbol/disassembly identify the remaining Raft FNV-1a loop: 283/283 BatchPut write_record_unsynced leaf samples (9.120% of all selected BatchPut samples) land at its loop sites. Point PUT has 48/51 method leaves there. This is sampled instruction-site attribution only, not all inclusive function CPU; optimized inline stacks alone do not isolate the hash. Engine frame_crc is a separate IEEE CRC32 path (561/3,103 BatchPut leaves, 18.079%).

Raw perf binaries, WAL/database data, full host/background process lists and tool binaries are excluded. Their original path/hash references remain where recorded; this bundle supports assessment of selected samples, full decoded interval coverage and source/process bindings, but cannot independently re-decode excluded perf.data. Decoder-verified data retention manifests are included without WAL payloads. Owned-process resource samples and lifetime records are included, not full host inventories.

The first packaging-authoring command failed on nested string quoting before writing package.py; its following invocation found no script. Both receipts remain in packaging-authoring-first-failure.json. This publication-only correction did not alter any runtime, decoder or original evidence.
'''
with(ROOT/'README.md').open('x')as f:f.write(readme)
file_inventory={str(q.relative_to(ROOT)):{'bytes':q.stat().st_size,'sha256':sha(q)}for q in sorted(ROOT.rglob('*'))if q.is_file()}
save(ROOT/'bundle-inventory.json',file_inventory)
summary={'complete':True,'source_and_copy_readback_unchanged':True,'original_files':len(index),'stored_original_bytes':sum(v['stored_bytes']for v in index.values()),'uncompressed_original_bytes':sum(v['original_bytes']for v in index.values()),'gzip_files':sum(v['encoding']=='gzip'for v in index.values()),'runtime_session':76156,'runtime_exit_code':0,'decoder_session':21261,'decoder_exit_code':0,'original_path_index_sha256':sha(ROOT/'original-path-index.json'),'bundle_inventory_sha256':sha(ROOT/'bundle-inventory.json'),'attribution_sha256':sha(ROOT/'write-record-attribution-first.json'),'completed_unix_ns':time.time_ns(),'scope':'Exact-byte CPU diagnostic evidence publication; raw perf/WAL/full host process inventories excluded; no new runtime or acceptance rerun.'}
save(ROOT/'BUNDLE.json',summary)
print(json.dumps(summary,indent=2));print('BUNDLE_SHA256',sha(ROOT/'BUNDLE.json'))
