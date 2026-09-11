from pathlib import Path
import hashlib,json,re,shutil,time
p=Path(__file__).resolve().parent;snapshot=p/'pending-first';base=Path('/tmp/kv9-owned-batch-broad-workloads-preparation')
def sha(f):return hashlib.sha256(f.read_bytes()).hexdigest()
failure={'phase':'ancillary pin metadata readback','exit_code':1,'tool_receipt':'a7e09b','exception':'FileNotFoundError: /tmp/kv9-read-credit-crc-release-rebuilt-first/kv9-batch-workload','cause':'The correctness workload is in the original helper workload/ subdirectory; first metadata writer assumed root layout. Measurement server/client paths and source predicates were already correct.','preserved_script':str(p/'finalize-pins-first.py'),'script_sha256':sha(p/'finalize-pins-first.py'),'runtime_test_or_build_executed':False,'correction':'Read actual workload/kv9-batch-workload. Do not rerun pin mutations or any runtime.'}
(p/'pin-readback-first-failure.json').write_text(json.dumps(failure,indent=2)+'\n')
replacement={'PENDING_CANDIDATE_BINARY_SHA256':'5464adee210f7cebbce7cd57d6e01bfe5a1ace42936edad24f57e63ff9cfbdd7','PENDING_CANDIDATE_MANIFEST_SHA256':'164b7999386b042ad2df814fea44fe93724ae15396e9f0cae0356a926fc3d1de','/tmp/kv9-read-credit-crc-release-first':'/tmp/kv9-read-credit-crc-release-rebuilt-first'}
pattern=re.compile('|'.join(re.escape(s) for s in sorted(replacement,key=len,reverse=True)))
records=[]
for name in json.loads((snapshot/'inventory.json').read_text()):
 expected=pattern.sub(lambda m:replacement[m[0]],(snapshot/name).read_text())
 if name in ['protocol.json','commands.json']:expected=expected.replace(sha(snapshot/'matched-driver.py'),sha(p/'matched-driver.py'))
 assert expected==(p/name).read_text(),name
 records.append({'name':name,'pending_sha256':sha(snapshot/name),'parent_sha256':sha(base/name),'final_sha256':sha(p/name)})
release=Path('/tmp/kv9-read-credit-crc-release-rebuilt-first');root_readback=Path('/tmp/kv9-read-credit-crc-root-first/release-rebuilt-readback.json');r=json.loads(root_readback.read_text())
assert r['accepted_for_runtime'] and r['source_clean'] and r['root_rebuild_exit_code']==0 and r['root_rebuild_session']==5651
assert r['build_manifest_sha256']==sha(release/'build.json')==replacement['PENDING_CANDIDATE_MANIFEST_SHA256']
assert r['server_sha256']==sha(release/'kv9')==replacement['PENDING_CANDIDATE_BINARY_SHA256']
assert r['workload_sha256']==sha(release/'workload/kv9-batch-workload')=='6447feee2ed494a0a2315bb757dc75c18b738b7f4462803ea30a76b30e7482ad'
assert r['source_files']==600 and r['revision']=='de37c71009e8199859b931e0037818f490e8d3f6'
assert r['source_tree_sha256']=='59924d6dfcc38dd3640866b424659cfc8319e886cf8d32745841faca4384e0bb'
shutil.copyfile(root_readback,p/'root-release-rebuilt-readback.json')
record={'accepted_pin_substitution':True,'runtime_executed':False,'source_revision':r['revision'],'source_files':600,'source_tree_sha256':r['source_tree_sha256'],'replacements':replacement,'root_readback':{'path':str(root_readback),'sha256':sha(root_readback)},'rejected_release_remains_retained':'/tmp/kv9-read-credit-crc-release-first','files':records,'created_ns':time.time_ns(),'ancillary_first_failure':str(p/'pin-readback-first-failure.json')}
(p/'final-pin-binding-second.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({'accepted':True,'driver_sha256':sha(p/'matched-driver.py'),'audit_sha256':sha(p/'audit.py'),'protocol_sha256':sha(p/'protocol.json'),'binding_sha256':sha(p/'final-pin-binding-second.json')},indent=2))
