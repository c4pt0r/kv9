"""Exact accepted native16 per-process CPU readback and bucket direction helpers."""
import math
CPUS=('client','voter-1','voter-2','voter-3')
def percent(old,new):return (new/old-1)*100 if old is not None and new is not None and old>0 else None

def bucket_direction(old,new):
 if old is None or new is None:return 'unavailable'
 if old==new:return 'same_bucket'
 if new['upper_ns']<old['lower_ns']:return 'new_lower_interval'
 if new['lower_ns']>old['upper_ns']:return 'new_higher_interval'
 return 'overlapping_intervals'

def cpu_from_samples(samples,coverage,report):
 begin=report['measurement_start_unix_ns'];end=begin+report['cohort_elapsed_ns'];inside=[s for s in samples if begin<=s['unix_ns']<=end]
 assert len(inside)==coverage['samples']>=10 and set(inside[0]['processes'])==set(CPUS)
 result={}
 for role in CPUS:
  first=inside[0]['processes'][role];last=inside[-1]['processes'][role]
  assert first['pid']==last['pid']and first['start_ticks']==last['start_ticks']
  duration_ns=last['observed_monotonic_ns']-first['observed_monotonic_ns'];delta=last['user_ticks']+last['system_ticks']-first['user_ticks']-first['system_ticks']
  assert duration_ns>0 and delta>=0
  seconds=duration_ns/1e9;cores=delta/100/seconds
  assert math.isclose(cores,coverage['cpu'][role]['cpu_cores'],rel_tol=1e-12)and seconds==coverage['cpu'][role]['sample_seconds']
  result[role]=dict(pid=first['pid'],start_ticks=first['start_ticks'],user_system_ticks=delta,sample_duration_ns=duration_ns,cpu_seconds=delta/100,sample_seconds=seconds,cpu_cores=cores)
 return result
