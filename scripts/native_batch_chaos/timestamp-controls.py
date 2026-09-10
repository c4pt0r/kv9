"""Read-only ISO timestamp parser controls for the retained failed fixture."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,importlib.util,json,hashlib,datetime
from pathlib import Path
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--output',type=Path,required=True);ap.add_argument('--timestamp',type=Path,required=True);args=ap.parse_args();A=Path(__file__).parent;O=args.output;O.mkdir(exist_ok=False)
s=importlib.util.spec_from_file_location('corrected_native',A/'check-native-windows.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
source=args.timestamp;actual=source.read_text().strip();expected=1789072484807368542
require_literal='2026-09-10T13:34:44,807368542-07:00';assert actual==require_literal, 'use the retained original timestamp fixture'
valid=[('actual-comma',actual,expected),('equivalent-dot',actual.replace(',','.'),expected),('equivalent-utc','2026-09-10T20:34:44.807368542Z',expected),('positive-offset','2026-09-10T21:34:44,807368542+01:00',expected),('whole-seconds','2026-09-10T20:34:44Z',expected-807368542),('one-digit','2026-09-10T20:34:44,8+00:00',expected-7368542),('one-nanosecond','2026-09-10T20:34:44,000000001+00:00',expected-807368541)]
invalid=[('missing-offset','2026-09-10T20:34:44,807368542'),('empty-fraction','2026-09-10T20:34:44,Z'),('excess-precision','2026-09-10T20:34:44,8073685421Z'),('mixed-separators','2026-09-10T20:34:44,.807368542Z'),('invalid-offset','2026-09-10T20:34:44+99:00'),('invalid-date','2026-19-99T20:34:44Z'),('trailing-junk','2026-09-10T20:34:44Z junk')]
rows=[]
try:
 for name,text,want in valid:
  p=O/(name+'.txt');p.write_text(text+'\n');got=m.iso_ns(p);assert got==want,(name,got,want);rows.append(dict(name=name,accepted=True,value_ns=got))
 for name,text in invalid:
  p=O/(name+'.txt');p.write_text(text+'\n')
  try:m.iso_ns(p)
  except ValueError as e:rows.append(dict(name=name,accepted=False,reason=str(e)))
  else:raise ValueError('invalid timestamp accepted: '+name)
 result=dict(accepted=True,scope=__doc__,source_timestamp=str(source),source_timestamp_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),adapter_sha256=hashlib.sha256((A/'check-native-windows.py').read_bytes()).hexdigest(),valid=7,invalid=7,controls=rows)
except BaseException as e:result=dict(accepted=False,failure=str(e),completed_controls=rows);raise
finally:(O/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items()if k!='controls'}))
