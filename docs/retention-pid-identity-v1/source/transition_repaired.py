"""Root-released retirement or exact original-path restore for one frozen cohort only."""
import sys,time
from authority import *
from verify import reconstruct

def verified(s,root,a):
 vp=canonical_path(a.verified_result);need(vp.parent.parent==root,'verification result scope');need(digest(vp)['sha256']==a.release_verified_sha256,'explicit reviewed verification hash')
 v=js(vp);need(v['complete']and v['state']=='VERIFIED'and v['selection_sha256']==a.selection_sha256 and v['code_pins_sha256']==a.code_pins_sha256 and v['all_children_reaped'],'independent verification incomplete/binding')
 retained_toolchain_ok(s,root,v['toolchain_sha256']);stage=js(root/'staged.json');need(digest(root/'staged.json')['sha256']==v['staged_sha256'],'stage changed')
 need(stage['toolchain_sha256']==v['toolchain_sha256'],'verified toolchain binding');rows={r['id']:r for r in stage['rows']};vr={r['id']:r for r in v['rows']}
 need(len(rows)==len(stage['rows'])==len(vr)==len(v['rows'])==s['target_count'] and set(rows)==set(vr)=={r['id']for r in s['pairs']},'complete verified target set')
 for rid,row in rows.items():
  need(row['patch']==vr[rid]['patch'] and vr[rid]['decoded_to_eof']and vr[rid]['original_compressed_exact'],'independent patch binding');check_patch(root,row)
 return rows

def second_cycle_gate(s,root,a):
 need(a.restore_result_sha256 and a.readback_result_sha256,'second COLD requires explicit restore and selected whole-cohort reader pins')
 rp=root/'restore-1/result.json';lp=root/'readback-1-repaired-first/result.json'
 need(digest(rp)['sha256']==a.restore_result_sha256 and digest(lp)['sha256']==a.readback_result_sha256,'second COLD actual results changed')
 restored=js(rp);legacy=js(lp)
 need(restored['complete']and restored['state']=='RESTORED'and repaired_reader_ok(legacy,s,a),'second COLD lacks actual exact restore/selected whole-cohort reader acceptance')
 need(restored['selection_sha256']==a.selection_sha256 and restored['verified_sha256']==a.release_verified_sha256,'restore lineage')
 expected={r['id']:r['identity']for r in restored['rows']};need(set(expected)=={r['target']for r in s['pairs']},'restored identity set')
 return expected

def main():
 ap=repair_args(args());ap.add_argument('action',choices=['retire','restore']);ap.add_argument('--cycle',type=int,choices=[1,2],required=True);ap.add_argument('--verified-result',required=True);ap.add_argument('--release-verified-sha256',required=True)
 ap.add_argument('--restore-result-sha256');ap.add_argument('--readback-result-sha256');a=ap.parse_args();need(a.cycle==2,'only final COLD and its exact restoration allowed in corrected transition');s=load(a);root,fd=lock(s);sess=session(root,s);mm=members(s)
 phase=phase_directory(root,a.action+'-'+str(a.cycle))
 try:
  rows=verified(s,root,a);preserved(s);no_references(s);authority=dict(selection_sha256=a.selection_sha256,verified_sha256=a.release_verified_sha256,cycle=a.cycle)
  expected={r['target']:mm[r['target']]['identity']for r in s['pairs']}
  if a.cycle==2:
   expected=second_cycle_gate(s,root,a)
   authority.update(restore_result_sha256=a.restore_result_sha256,readback_result_sha256=a.readback_result_sha256)
  if a.action=='retire':
   # Refuse all pre-existing collisions before the first unlink. Absent originals need this cycle's durable intent.
   for pair in s['pairs']:
    path=Path(mm[pair['target']]['path']);intent=phase/(pair['id']+'.retire-intent.json')
    if absent(path):need(intent.exists(),'missing original without durable authorized intent')
    else:object_ok(mm[pair['target']],expected[pair['target']])
   receipts=[]
   for pair in s['pairs']:
    sess.guard();preserved(s);object_ok(mm[pair['base']]);check_patch(root,rows[pair['id']]);ref=no_references(s)
    target=mm[pair['target']];intent=phase/(pair['id']+'.retire-intent.json')
    r=retire_one(Path(target['path']),intent,expected[pair['target']],target['compressed'],authority)
    ack=phase/(pair['id']+'.json')
    if absent(ack):durable_json(ack,dict(id=pair['id'],**r,reference_observation=ref))
    receipts.append(js(ack))
   need(all(absent(Path(mm[p['target']]['path']))for p in s['pairs']),'COLD target still resident');preserved(s)
   doc=dict(complete=True,state='COLD',selection_sha256=a.selection_sha256,verified_sha256=a.release_verified_sha256,cycle=a.cycle,rows=receipts,accounting=accounting(s),resource=sess.guard(),legacy_reader_requires_original_object_restoration=True)
  else:
   need(a.cycle in (1,2),'bounded first or second COLD restoration only')
   # A partially retired phase may be rolled back; every missing object still needs the released intent.
   retired=root/('retire-'+str(a.cycle));need(retired.is_dir(),'no retirement intent root');restored=[]
   for pair in s['pairs']:
    target=mm[pair['target']];dst=Path(target['path']);ack=phase/(pair['id']+'.json');intent=phase/(pair['id']+'.publish-intent.json')
    ri=retired/(pair['id']+'.retire-intent.json')
    if ack.exists():
     r=js(ack);need(identity(dst)==r['identity'],'restored identity changed');check(dst,target['compressed']);restored.append(r);continue
    if intent.exists():
     r=js(intent);pub=publish(Path(r['source']),dst,intent,target['compressed']);out=dict(id=pair['id'],identity=pub['identity'],original_identity=target['identity'],original_bytes_exact=True,recovered_publication=True)
    elif not absent(dst):
     # Unretired original during a partial rollback must still have its original exact identity.
     object_ok(target,expected[pair['target']]);out=dict(id=pair['id'],identity=identity(dst),original_identity=target['identity'],original_bytes_exact=True,never_retired=True)
    else:
     need(ri.exists()and js(ri)['authority']==authority,'missing object without same-release retirement intent')
     w=work(root,'restore',pair['id']);encoded,scratch=reconstruct(sess,s,pair,rows[pair['id']],w)
     need(identity(encoded)['allocated_bytes']<=target['compressed']['bytes']+MIB,'external restore allowance exceeded')
     st=target['identity'];os.chmod(encoded,stat.S_IMODE(st['mode']));os.chown(encoded,st['uid'],st['gid']);os.utime(encoded,ns=(st['mtime_ns'],st['mtime_ns']))
     f=os.open(encoded,os.O_RDONLY|os.O_NOFOLLOW)
     try:os.fsync(f)
     finally:os.close(f)
     pub=publish(encoded,dst,intent,target['compressed']);out=dict(id=pair['id'],identity=pub['identity'],original_identity=st,original_bytes_exact=True,inode_ctime_preserved=False)
     remove_verified_work(w,scratch)
    durable_json(ack,out);restored.append(out);sess.guard()
   preserved(s);no_references(s)
   need(len(restored)==s['target_count'],'restore target count')
   doc=dict(complete=True,state='RESTORED',selection_sha256=a.selection_sha256,verified_sha256=a.release_verified_sha256,rows=restored,children=sess.records,all_children_reaped=all(r['reaped']and r['exit_code']==0 for r in sess.records),accounting=accounting(s),resource=sess.guard(),selected_reader_acceptance_pending=True)
  doc['reconciliation']=reconciliation_binding(s,a)
  if absent(phase/'result.json'):result(phase,'result.json',doc)
  else:need(js(phase/'result.json')['complete'],'existing incomplete phase result');print(json.dumps(dict(complete=True,resumed_completed_phase=str(phase),result_sha256=digest(phase/'result.json')['sha256'])))
  return 0
 except BaseException as exc:result(phase,'failure-'+str(time.time_ns())+'.json',dict(complete=False,error=repr(exc),children=sess.records,scope='All partial patches, originals, restore outputs and durable intents retained; no broad cleanup.'));return 1
 finally:os.close(fd)
if __name__=='__main__':sys.exit(main())
