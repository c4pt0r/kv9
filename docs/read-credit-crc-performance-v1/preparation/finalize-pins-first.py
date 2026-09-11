from pathlib import Path
import difflib,hashlib,json,re,shutil,time
p=Path(__file__).resolve().parent;snapshot=p/'pending-first';base=Path('/tmp/kv9-owned-batch-broad-workloads-preparation')
old=json.loads((snapshot/'inventory.json').read_text())
def sha(f):return hashlib.sha256(f.read_bytes()).hexdigest()
for name,row in old.items():assert sha(p/name)==sha(snapshot/name)==row['sha256'],name
# Preserve the documentation and diffs that described the unbound preparation.
for name in ['README.md','pending-handoff-first.json']:
 target=snapshot/name;assert not target.exists();shutil.copyfile(p/name,target)
for name in old:
 target=snapshot/(name+'.diff');target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p/(name+'.diff'),target)
replacement={
 'PENDING_CANDIDATE_BINARY_SHA256':'5464adee210f7cebbce7cd57d6e01bfe5a1ace42936edad24f57e63ff9cfbdd7',
 'PENDING_CANDIDATE_MANIFEST_SHA256':'164b7999386b042ad2df814fea44fe93724ae15396e9f0cae0356a926fc3d1de',
 '/tmp/kv9-read-credit-crc-release-first':'/tmp/kv9-read-credit-crc-release-rebuilt-first'}
pattern=re.compile('|'.join(re.escape(s) for s in sorted(replacement,key=len,reverse=True)))
old_driver=sha(p/'matched-driver.py')
for name in old:
 (p/name).write_text(pattern.sub(lambda m:replacement[m[0]],(snapshot/name).read_text()))
new_driver=sha(p/'matched-driver.py')
for name in ['protocol.json','commands.json']:
 text=(p/name).read_text();assert text.count(old_driver)==1;(p/name).write_text(text.replace(old_driver,new_driver))
records=[]
for name in old:
 target=p/name
 assert 'PENDING_CANDIDATE' not in target.read_text() and '/tmp/kv9-read-credit-crc-release-first' not in target.read_text()
 (p/(name+'.final.diff')).write_text(''.join(difflib.unified_diff((snapshot/name).read_text().splitlines(True),target.read_text().splitlines(True),fromfile=str(snapshot/name),tofile=str(target))))
 (p/(name+'.diff')).write_text(''.join(difflib.unified_diff((base/name).read_text().splitlines(True),target.read_text().splitlines(True),fromfile=str(base/name),tofile=str(target))))
 records.append({'name':name,'pending_sha256':sha(snapshot/name),'parent_sha256':sha(base/name),'final_sha256':sha(target)})
release=Path('/tmp/kv9-read-credit-crc-release-rebuilt-first');root_readback=Path('/tmp/kv9-read-credit-crc-root-first/release-rebuilt-readback.json')
r=json.loads(root_readback.read_text());assert r['accepted_for_runtime'] and r['source_clean'] and r['root_rebuild_exit_code']==0 and r['root_rebuild_session']==5651
assert r['build_manifest_sha256']==sha(release/'build.json')==replacement['PENDING_CANDIDATE_MANIFEST_SHA256']
assert r['server_sha256']==sha(release/'kv9')==replacement['PENDING_CANDIDATE_BINARY_SHA256']
assert r['workload_sha256']==sha(release/'kv9-batch-workload')=='6447feee2ed494a0a2315bb757dc75c18b738b7f4462803ea30a76b30e7482ad'
assert r['source_files']==600 and r['revision']=='de37c71009e8199859b931e0037818f490e8d3f6'
assert r['source_tree_sha256']=='59924d6dfcc38dd3640866b424659cfc8319e886cf8d32745841faca4384e0bb'
shutil.copyfile(root_readback,p/'root-release-rebuilt-readback.json')
record={'accepted_pin_substitution':True,'runtime_executed':False,'source_revision':r['revision'],'source_files':600,'source_tree_sha256':r['source_tree_sha256'],'replacements':replacement,'root_readback':{'path':str(root_readback),'sha256':sha(root_readback)},'rejected_release_remains_retained':'/tmp/kv9-read-credit-crc-release-first','files':records,'created_ns':time.time_ns()}
(p/'final-pin-binding-first.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({'accepted':True,'driver_sha256':sha(p/'matched-driver.py'),'audit_sha256':sha(p/'audit.py'),'protocol_sha256':sha(p/'protocol.json'),'binding_sha256':sha(p/'final-pin-binding-first.json')},indent=2))
