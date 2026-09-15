#!/usr/bin/env python3
"""One bounded metadata/history archive, retaining originals and failures."""
import gzip,hashlib,json,os,stat,tarfile,time,traceback
from pathlib import Path
P=Path(__file__).resolve().parent
START=time.monotonic()
def pin(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return {'path':str(p),'bytes':p.stat().st_size,'sha256':h.hexdigest()}
def save(n,x):
 with (P/n).open('x') as f:json.dump(x,f,indent=2,sort_keys=True);f.write('\n');f.flush();os.fsync(f.fileno())
def guard():
 assert time.monotonic()-START<=120,'publication deadline'
 for p in [P,Path('/')]:
  s=os.statvfs(p);assert s.f_bavail*s.f_frsize>=8*1024**3,'publication floor'
 assert sum(p.stat().st_blocks*512 for p in P.rglob('*') if p.is_file())<1024**3,'publication added allocation'
def add(t,name,p,expected):
 guard();s=p.lstat();assert stat.S_ISREG(s.st_mode) and not p.is_symlink()
 got=pin(p);assert all(got[k]==expected[k]for k in ('bytes','sha256')),'changed original '+str(p)
 m=tarfile.TarInfo(name);m.size=got['bytes'];m.mode=0o644;m.mtime=0;m.uid=m.gid=0
 with p.open('rb') as f:t.addfile(m,f)
 assert p.lstat()==s,'source metadata changed '+str(p)
try:
 selection=json.loads((P/'selection.json').read_text());assert selection['complete']
 archive=P/'evidence.tar.gz'
 with archive.open('xb') as out:
  with gzip.GzipFile(filename='',mode='wb',fileobj=out,mtime=0,compresslevel=6) as gz:
   with tarfile.open(fileobj=gz,mode='w|',format=tarfile.USTAR_FORMAT) as t:
    add(t,'manifest.json',P/'selection.json',pin(P/'selection.json'))
    for r in selection['files']:add(t,r['member'],Path(r['path']),r)
  out.flush();os.fsync(out.fileno())
 parts=[]
 with archive.open('rb') as f:
  i=0
  while b:=f.read(2*1024**2):
   p=P/('evidence.tar.gz.part-%03d'%i)
   with p.open('xb') as out:out.write(b);out.flush();os.fsync(out.fileno())
   parts.append(pin(p));i+=1;guard()
 save('package-result.json',{'complete':True,'selection':pin(P/'selection.json'),'archive':pin(archive),'parts':parts,'gzip_member_count':1,'tar_file_members':selection['member_count']+1,'selected_original_bytes':selection['original_bytes'],'elapsed_seconds':time.monotonic()-START,'originals_retained':True,'scope':'Reporting compression only; no database/codec acceptance replay.'})
 print((P/'package-result.json').read_text())
except BaseException as e:
 save('package-failure.json',{'complete':False,'error':repr(e),'traceback':traceback.format_exc(),'partial_outputs_retained':True});raise
