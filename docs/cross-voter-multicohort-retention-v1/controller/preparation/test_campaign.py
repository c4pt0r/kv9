"""Root-only bounded controller controls: synthetic metadata/mocked calls, no codecs."""
import copy,fcntl,json,os,tempfile,types,unittest
from pathlib import Path
from unittest.mock import patch
import campaign as c

class CampaignGuards(unittest.TestCase):
 def proofs(self):
  orig=dict(bytes=3,sha256='raw');encoded=dict(bytes=2,sha256='encoded');p=dict(id='t',base='b',target='t',patch_limit=100)
  s=dict(target_count=1,original_object_count=2,pairs=[p],members=[dict(id='b',original=orig,compressed=encoded),dict(id='t',original=orig,compressed=encoded)])
  patchrow=dict(path='/patch',identity=dict(inode=1),pin=dict(bytes=1,sha256='patch'))
  a=dict(rows=[dict(id='t',pair=p,patch=patchrow)]);v=dict(rows=[dict(id='t',base='b',patch=patchrow,original=orig,compressed=encoded,decoded_to_eof=True,original_compressed_exact=True)])
  return s,a,v
 def test_exact_verification_proof_binding(self):
  s,a,v=self.proofs();self.assertEqual(len(c.pair_proofs(s,a,v)[0]),1)
  mutations=[lambda d:d['rows'].clear(),lambda d:d['rows'].append(copy.deepcopy(d['rows'][0])),lambda d:d['rows'][0].update(id='other'),lambda d:d['rows'][0].update(base='other'),lambda d:d['rows'][0].update(original=dict(bytes=4,sha256='raw')),lambda d:d['rows'][0].update(compressed=dict(bytes=2,sha256='wrong')),lambda d:d['rows'][0].update(patch={}),lambda d:d['rows'][0].update(decoded_to_eof=False),lambda d:d['rows'][0].update(original_compressed_exact=False)]
  for mutate in mutations:
   bad=copy.deepcopy(v);mutate(bad)
   with self.assertRaises((RuntimeError,KeyError)):c.pair_proofs(s,a,bad)
 def test_stage_pair_change_refused(self):
  s,a,v=self.proofs();a=copy.deepcopy(a);a['rows'][0]['pair']['target']='elsewhere'
  with self.assertRaises(RuntimeError):c.pair_proofs(s,a,v)
 def test_unknown_failed_live_child_refused(self):
  good=dict(pid=9,start_ticks=10,boot_id='boot',complete=True,exit_code=0,reaped=True)
  with patch.object(c,'lifetime',lambda pid:None):
   c.outcome(good)
   for key,val in [('complete',False),('exit_code',1),('reaped',False)]:
    bad=dict(good);bad[key]=val
    with self.assertRaises(RuntimeError):c.outcome(bad)
  with patch.object(c,'lifetime',lambda pid:dict(pid=9,start_ticks=10,boot_id='boot')):
   with self.assertRaisesRegex(RuntimeError,'same owned'):c.outcome(good)
  with patch.object(c,'lifetime',lambda pid:dict(pid=9,start_ticks=11,boot_id='boot')):c.outcome(good)
 def test_only_exact_completed_prefix_resumes(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   p=Path(d);self.assertEqual(c.resume_prefix(p),([],1));(p/'001').mkdir()
   with self.assertRaisesRegex(RuntimeError,'incomplete/unknown'):c.resume_prefix(p)
   (p/'001/complete.json').write_text(json.dumps(dict(complete=True,ordinal=1)));self.assertEqual(c.resume_prefix(p)[1],2)
   (p/'003').mkdir()
   with self.assertRaisesRegex(RuntimeError,'noncontiguous'):c.resume_prefix(p)
 def test_failed_prefix_never_retried(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   p=Path(d);(p/'001').mkdir();(p/'001/complete.json').write_text(json.dumps(dict(complete=False,ordinal=1)))
   with self.assertRaises(RuntimeError):c.resume_prefix(p)
 def test_actual_space_stop_and_finite_exhaustion(self):
  self.assertIsNone(c.stop_reason(c.STOP-1,95));self.assertEqual(c.stop_reason(c.STOP,95),'actual_available_target');self.assertEqual(c.stop_reason(c.STOP-1,96),'finite_exhaustion')
 def test_serial_owner_lock(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   p=Path(d)/'lock';a=os.open(p,os.O_CREAT|os.O_RDWR,0o600);b=os.open(p,os.O_RDWR)
   try:
    fcntl.flock(a,fcntl.LOCK_EX|fcntl.LOCK_NB)
    with self.assertRaises(BlockingIOError):fcntl.flock(b,fcntl.LOCK_EX|fcntl.LOCK_NB)
   finally:os.close(a);os.close(b)
 def test_changed_bytes_and_metadata_rejected(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   p=Path(d)/'x';p.write_bytes(b'abc');pin=c.pin(p);c.check(p,pin);p.write_bytes(b'abd')
   with self.assertRaises(RuntimeError):c.check(p,pin)
   p.unlink();p.symlink_to('/absent')
   with self.assertRaises(RuntimeError):c.pin(p)
 def test_floor_output_and_deadline_guards(self):
  with patch.object(c,'available',lambda:c.FLOOR),patch.object(c,'allocated',lambda root:c.CAP):c.resource_guard()
  with patch.object(c,'available',lambda:c.FLOOR-1),patch.object(c,'allocated',lambda root:0):
   with self.assertRaises(RuntimeError):c.resource_guard()
  with patch.object(c,'available',lambda:c.FLOOR),patch.object(c,'allocated',lambda root:c.CAP+1):
   with self.assertRaises(RuntimeError):c.resource_guard()
  with patch.object(c,'available',lambda:c.FLOOR),patch.object(c,'allocated',lambda root:0),patch.object(c.time,'monotonic',lambda:11):
   with self.assertRaises(RuntimeError):c.resource_guard(0,10)
 def test_parent_death_gap_and_failure(self):
  libc=types.SimpleNamespace(prctl=lambda *a:0)
  with patch.object(c.ctypes,'CDLL',lambda *a,**k:libc),patch.object(c.os,'getppid',lambda:12),patch.object(c.os,'kill')as kill:
   c.pdeath(12);kill.assert_not_called();c.pdeath(13);kill.assert_called_once()
  libc.prctl=lambda *a:1
  with patch.object(c.ctypes,'CDLL',lambda *a,**k:libc):
   with self.assertRaises(RuntimeError):c.pdeath(12)
 def test_zero_finish_requires_actual_matching_terminal(self):
  result='/zero/retire-2/result.json';v='/zero/verification-first/result.json';t=dict(session_id=56949,initial_chunk='aa2cc1',final_chunk='ba03d0',exit_code=0,result_path=result,result_sha256='cold');child=dict(complete=True,exit_code=0,result_path=result,result_sha256='cold',argv=[str(c.OLD/'run.py'),'--mode','finish','--ordinal','0','--release-verified-sha256',c.ZERO_VERIFIED])
  group=dict(rows=[dict(root='/zero')]);pins={'/terminal':'t','/child':'c',result:'cold',v:c.ZERO_VERIFIED}
  def run(term,ch):
   with patch.object(c,'pin',lambda p:dict(sha256=pins[str(p)])),patch.object(c,'js',lambda p:term if str(p)=='/terminal'else ch),patch.object(c,'completed',lambda *a:dict(complete=True)):
    return c.zero_gate(group,'/terminal','t','/child','c')
  self.assertTrue(run(t,child)['complete'])
  for key,val in [('exit_code',1),('result_sha256','wrong'),('session_id',0),('final_chunk','')]:
   bad=dict(t);bad[key]=val
   with self.assertRaises(RuntimeError):run(bad,child)
  bad=dict(child,complete=False)
  with self.assertRaises(RuntimeError):run(t,bad)
 def test_no_automatic_existing_phase_restart(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   root=Path(d);(root/'001').mkdir();(root/'001/stage-verify').mkdir()
   with patch.object(c,'OUT',root),patch.object(c.subprocess,'Popen')as spawn:
    with self.assertRaisesRegex(RuntimeError,'never restart'):c.invoke(1,'stage-verify')
    spawn.assert_not_called()
class ReleaseClosure(unittest.TestCase):
 def fixture(self):
  base=dict(bytes=3,device=1,inode=2,mode=33188);target=dict(bytes=4,device=1,inode=3,mode=33188)
  pair=dict(id='t',base='b',target='t',patch_limit=100)
  raw=dict(bytes=10,sha256='raw');enc=dict(bytes=4,sha256='encoded')
  s=dict(root='/tx',codec=dict(path='/usr/bin/zstd'),target_count=1,original_object_count=2,pairs=[pair],members=[dict(id='b',identity=base,original=raw,compressed=enc,decode_argv=['/usr/bin/zstd','-d'],encode_argv=['/usr/bin/zstd','-c']),dict(id='t',identity=target,original=raw,compressed=enc,decode_argv=['/usr/bin/zstd','-d'],encode_argv=['/usr/bin/zstd','-c'])])
  patchrow=dict(path='/tx/patches/t.zst',pin=dict(bytes=1,sha256='patch'),identity={})
  a=dict(rows=[dict(id='t',pair=pair,patch=patchrow,children=[dict(label=x)for x in ['base-t','target-t','patch-t']])])
  v=dict(rows=[dict(id='t',base='b',patch=patchrow,original=raw,compressed=enc,decoded_to_eof=True,original_compressed_exact=True)],children=[dict(label=x)for x in ['independent-base-t','independent-patch-t','original-encode-t']])
  return c.codec_roles(s,a,v)[0]
 def record(self,e):
  a={k:copy.deepcopy(e[k])for k in ['label','argv','output','stderr','limit_bytes','decoded']};a.update(started_ns=1,rlimit_as_bytes=512*c.MIB,cpu_affinity=sorted(c.CPUS),resource_before=dict(available_bytes=c.FLOOR))
  if e['stdin']is not None:a.update(stdin_mode='regular_file_descriptor',stdin_identity=dict(device=1,inode=2,mode=33188,**{k:v for k,v in e['stdin'].items()if k not in ['device','inode','mode']}));a['stdin_identity'].update(e['stdin'])
  start=dict(a,pid=999999999,start_ticks=2,boot_id='boot');intent=dict(a,parent_pid=1,reserved_decoded_bytes=e['limit_bytes']if e['decoded']else 0);final=dict(start,complete=True,exit_code=0,reaped=True,error=None,ended_ns=2,output_bytes=e['output_bytes'])
  return final,intent,start
 def test_all_six_exact_roles_and_mutated_self_consistent_argv(self):
  roles=self.fixture();self.assertEqual(len(roles),6)
  with patch.object(c,'lifetime',lambda pid:None):
   for e in roles:
    r,a,b=self.record(e);c.codec_receipt(e,r,a,b,copy.deepcopy(r))
    for key,value in [('argv',['/wrong-codec']),('output','/wrong/output'),('output_bytes',e['output_bytes']+1),('limit_bytes',e['limit_bytes']+1)]:
     x,y,z=copy.deepcopy((r,a,b));x[key]=value
     if key in y:y[key]=value
     if key in z:z[key]=value
     with self.assertRaises(RuntimeError):c.codec_receipt(e,x,y,z,copy.deepcopy(x))
 def test_embedded_mismatch_and_orphan_sidecar(self):
  roles=self.fixture();e=roles[0];r,a,b=self.record(e)
  with patch.object(c,'lifetime',lambda pid:None):
   with self.assertRaises(RuntimeError):c.codec_receipt(e,r,a,b,{})
  finals=[x['output']+'.child.json'for x in roles];intents=[x['output']+'.child-intent.json'for x in roles];starts=[x['output']+'.child-start.json'for x in roles]
  c.sidecar_closure(roles,finals,intents,starts)
  with self.assertRaises(RuntimeError):c.sidecar_closure(roles,finals,intents+['/orphan.child-intent.json'],starts)
  with self.assertRaises(RuntimeError):c.sidecar_closure(roles,finals[:-1],intents,starts)
 def test_global_transaction_and_handle_prefix(self):
  with tempfile.TemporaryDirectory(prefix='kv9-campaign-control-')as d:
   p=Path(d);execution=p/'exec';execution.mkdir();rows=[dict(ordinal=i,root=str(p/f'tx{i}'))for i in range(3)];g=dict(rows=rows);Path(rows[0]['root']).mkdir();(execution/'000').mkdir()
   with patch.object(c,'EXEC',execution):
    c.global_prefix(g,0);Path(rows[2]['root']).mkdir();(execution/'002').mkdir()
    with self.assertRaises(RuntimeError):c.global_prefix(g,0)
    Path(rows[2]['root']).rmdir();(execution/'002').rmdir();(execution/'001').mkdir()
    with self.assertRaises(RuntimeError):c.global_prefix(g,0)
    Path(rows[1]['root']).mkdir();c.global_prefix(g,1)

if __name__=='__main__':unittest.main(verbosity=2)
