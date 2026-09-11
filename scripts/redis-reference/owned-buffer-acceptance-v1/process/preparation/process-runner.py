import importlib.util,json,os,sys
from pathlib import Path
sys.dont_write_bytecode=True
source=Path('/tmp/kv9-owned-apply-batch')
sys.path.insert(0,str(source/'scripts'))
spec=importlib.util.spec_from_file_location('native_e2e',source/'scripts/native-batch-e2e.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
original=m.StreamingFixture.__init__
def initialize(self,*args,**kwargs):
 original(self,*args,**kwargs)
 placement=dict(client_cpus=[6,7],server_cpus=list(range(8,16))+list(range(22,32)),separated=True,exclusive_host=False)
 self.placement=placement;self.record['placement']=placement
m.StreamingFixture.__init__=initialize
assert 'TOKIO_WORKER_THREADS' not in os.environ
m.main()
