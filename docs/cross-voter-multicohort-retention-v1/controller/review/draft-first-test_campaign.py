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
if __name__=='__main__':unittest.main(verbosity=2)
