import hashlib, json, struct, zlib
from pathlib import Path
root=Path(__file__).resolve().parent
raw=(root/'original-segment.wal').read_bytes()
meta=json.loads((root/'input.json').read_text())
assert len(raw)==meta['bytes'] and hashlib.sha256(raw).hexdigest()==meta['sha256']
header=raw[:60]
assert header[:8]==b'KV9SEG01' and zlib.crc32(header[:56])==int.from_bytes(header[56:],'little')
at=60; rows=[]; batches=[]; previous=int.from_bytes(header[48:56],'little')
while at<len(raw):
 h=raw[at:at+32]; size=int.from_bytes(h[4:8],'little')
 assert h[:4]==b'KV9R' and zlib.crc32(h[:28])==int.from_bytes(h[28:32],'little') and h[8]==1 and h[9:12]==bytes(3)
 index=int.from_bytes(h[20:28],'little'); assert index>previous; previous=index
 payload=raw[at+32:at+32+size]; crc=raw[at+32+size:at+36+size]
 assert len(payload)==size and len(crc)==4
 assert zlib.crc32(payload,zlib.crc32(h[:28],zlib.crc32(header[:56])))==int.from_bytes(crc,'little')
 n=int.from_bytes(payload[:4],'little'); cursor=4; keys=[]
 for i in range(n):
  tag,cf=payload[cursor:cursor+2]; cursor+=2; assert tag in (0,1) and cf in (0,1,2)
  length=int.from_bytes(payload[cursor:cursor+4],'little');cursor+=4
  key=payload[cursor:cursor+length];cursor+=length; assert len(key)==length
  keys.append((cf,key))
  if tag==0:
   length=int.from_bytes(payload[cursor:cursor+4],'little');cursor+=4+length
 assert cursor==len(payload)
 rows.append(dict(index=index,mutations=n,unique_keys=len(set(keys)),redundant=n-len(set(keys)),descending_adjacent_pairs=sum(a>b for a,b in zip(keys,keys[1:]))))
 batches.append(payload);at+=36+size
assert at==len(raw)
# Payload-only corpus for a memory-index microbenchmark. Framing: u32 length + exact encoded batch.
with (root/'batches.bin').open('xb') as f:
 for b in batches: f.write(struct.pack('<I',len(b)));f.write(b)
r=dict(input=meta,frames=len(rows),mutations=sum(r['mutations'] for r in rows),redundant=sum(r['redundant'] for r in rows),min_batch=min(r['mutations'] for r in rows),max_batch=max(r['mutations'] for r in rows),rows=rows,scope='One preselected retained segment; includes all of its frames, not the entire timed history. CRC/position checks are offline format checks, not Raft authority.')
r['redundant_percent']=100*r['redundant']/r['mutations']
(root/'corpus-analysis.json').write_text(json.dumps(r,indent=2)+'\n')
print(json.dumps({k:v for k,v in r.items() if k not in ('rows','input')}))
