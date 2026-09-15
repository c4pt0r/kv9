"""Run the exactly bound whole-cohort readback representation after exact-path restoration."""
import resource,signal,sys,time
from authority import *

def main():
 ap=repair_args(args());ap.add_argument('--cycle',type=int,choices=[1,2],required=True);ap.add_argument('--restore-result-sha256',required=True);a=ap.parse_args();s=load(a);need(not a.retained_toolchain_sha256,'whole-cohort readback requires the original reader system codec path');root,fd=lock(s);sess=session(root,s)
 dest=canonical_path(root/('readback-'+str(a.cycle)+'-repaired-first'));need(absent(dest),'fresh readback output required');dest.mkdir(mode=0o700);fsync_dir(root)
 records={};logical={};decoders=[]
 try:
  restore=root/('restore-'+str(a.cycle))/'result.json';need(digest(restore)['sha256']==a.restore_result_sha256,'actual restoration pin')
  r=js(restore);need(r['complete']and r['state']=='RESTORED'and r['selection_sha256']==a.selection_sha256 and r['all_children_reaped'],'restore incomplete/binding')
  need(len(r['rows'])==s['target_count'] and len({x['id']for x in r['rows']})==s['target_count'],'exact restored target set')
  mm=members(s)
  for x in r['rows']:object_ok(mm[x['id']],x['identity'])
  preserved(s);no_references(s);sess.guard(16*MIB)
  charged=s['logical_cohort_bytes'];
  if s['group_ordinal']==13:charge_gate(sess.decoded,charged,s['limits']['cumulative_decoded_bytes'],a.cycle)
  need(sess.decoded+charged<=s['limits']['cumulative_decoded_bytes'],'readback whole-cohort decoded reservation exceeds cumulative limit')
  # The full amount is durably charged even if this readback attempt later fails or is interrupted.
  durable_json(dest/'decode-reservation.json',dict(charged_decoded_bytes=charged,previous_cumulative_decoded_bytes=sess.decoded,restore_result_sha256=a.restore_result_sha256,scope='Full cohort is conservatively charged on failure or interruption; never only successful decoder rows.'))
  sess.decoded+=charged
  reader_pin=effective_reader(s);reader=module('selected_whole_cohort_retention_reader',reader_pin['path'],reader_pin)
  original_children=js(Path(s['cohort'])/'cleanup.json')['children']
  resource.setrlimit(resource.RLIMIT_AS,(512*MIB,512*MIB))
  def timeout(signum,frame):raise TimeoutError('1800 second whole-cohort readback deadline')
  signal.signal(signal.SIGALRM,timeout);signal.alarm(1800)
  observed=reader.verify(Path(s['fixture']),original_children,records,logical,decoders)
  signal.alarm(0)
  need(observed['files']==s['original_object_count'] and observed['logical_bytes']==s['logical_cohort_bytes']and observed['compressed_bytes']==s['encoded_cohort_bytes'],'whole-cohort readback result scope')
  preserved(s)
  result(dest,'result.json',dict(complete=True,selection_sha256=a.selection_sha256,restore_result_sha256=a.restore_result_sha256,preparation_inventory_sha256=a.preparation_inventory_sha256,**reader_binding(s),observed=observed,records=records,logical_original_inventory=logical,decoder_receipts=decoders,charged_decoded_bytes=charged,accounting=accounting(s),scope='Selected reader verify() with all original catalog/receipt/time/child/count/cap/exact-object/decode predicates. Fresh inode/ctime is not historical inode residency. No campaign timing acceptance rerun.'))
  return 0
 except BaseException as exc:
  signal.alarm(0);result(dest,'result.json',dict(complete=False,error=repr(exc),restore_result_sha256=a.restore_result_sha256,decoder_receipts=decoders,scope='Full predeclared readback decoded reservation remains charged; all restored originals and transaction evidence preserved.'));return 1
 finally:os.close(fd)
if __name__=='__main__':sys.exit(main())
