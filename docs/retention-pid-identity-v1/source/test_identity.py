"""Metadata-only controls of the actual corrected producer ownership expression."""
import argparse, ast, copy, hashlib, importlib.util, io, json, sys, time, unittest
from pathlib import Path
from unittest.mock import patch
P=Path(__file__).resolve().parent
B=json.loads((P/'reader-bindings.json').read_text())
COLLISION=json.loads((P/'collision.json').read_text())
BOOT=COLLISION['recorded_producer']['boot_id']
OTHER_BOOT='00000000-0000-0000-0000-000000000001'
READERS=[]
for row in B['rows']:
    spec=importlib.util.spec_from_file_location(row['family'].replace('-','_'),row['corrected_reader']['path'])
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    tree=ast.parse(Path(row['corrected_reader']['path']).read_text())
    expressions=[n for n in ast.walk(tree) if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and
        len(n.value.args)==2 and isinstance(n.value.args[1],ast.Constant) and n.value.args[1].value=='producer codec ownership/exit']
    assert len(expressions)==1
    code=compile(ast.fix_missing_locations(ast.Module(body=expressions,type_ignores=[])),row['corrected_reader']['path'],'exec')
    READERS.append((row,module,code))

def proc_stat(pid,ticks):
    fields=['S']+['0']*18+[str(ticks)]
    return str(pid)+' (reader (nested) comm) '+' '.join(fields)+'\n'

def fake_path(stat_value=None,boot_values=None):
    values=list(boot_values or [BOOT,BOOT]);counter=[0]
    class Fake:
        def __init__(self,path):self.path=str(path)
        def read_text(self):
            if self.path=='/proc/sys/kernel/random/boot_id':
                index=min(counter[0],len(values)-1);counter[0]+=1;value=values[index]
            elif self.path==f'/proc/{COLLISION["recorded_producer"]["pid"]}/stat':
                value=stat_value if stat_value is not None else FileNotFoundError('absent synthetic process')
            else:raise AssertionError('unexpected read '+self.path)
            if isinstance(value,BaseException):raise value
            return value
    return Fake

def execute(module,code,row,path):
    argv=[module.CODEC,'-q','--no-progress','--single-thread','-d','--memory=64MB','-c']
    with patch.object(module,'Path',path):
        exec(code,dict(module.__dict__,c=row,mode='decode',argv=argv))

class IdentityControls(unittest.TestCase):
    def each(self):return READERS
    def test_absent_process(self):
        for r,m,c in self.each():
            with self.subTest(family=r['family']):execute(m,c,copy.deepcopy(COLLISION['recorded_producer']),fake_path())
    def test_same_pid_different_start(self):
        old=COLLISION['recorded_producer']
        for r,m,c in self.each():
            with self.subTest(family=r['family']):execute(m,c,copy.deepcopy(old),fake_path(proc_stat(old['pid'],old['start_ticks']+1)))
    def test_different_boot(self):
        old=COLLISION['recorded_producer']
        for r,m,c in self.each():
            with self.subTest(family=r['family']):execute(m,c,copy.deepcopy(old),fake_path(proc_stat(old['pid'],old['start_ticks']),[OTHER_BOOT,OTHER_BOOT]))
    def test_same_live_lifetime_rejected(self):
        old=COLLISION['recorded_producer']
        for r,m,c in self.each():
            with self.subTest(family=r['family']),self.assertRaisesRegex(ValueError,'producer codec ownership/exit'):
                execute(m,c,copy.deepcopy(old),fake_path(proc_stat(old['pid'],old['start_ticks'])))
    def test_missing_malformed_record_refused(self):
        values={'pid':[None,False,0,-1,'1272867'],'start_ticks':[None,True,0,-1,'171408954'],
                'boot_id':[None,7,'','not-a-boot-id','0'*36]}
        for r,m,c in self.each():
            for key,bad in values.items():
                for value in bad:
                    row=copy.deepcopy(COLLISION['recorded_producer']);row[key]=value
                    with self.subTest(family=r['family'],key=key,value=value),self.assertRaises(ValueError):execute(m,c,row,fake_path())
                row=copy.deepcopy(COLLISION['recorded_producer']);del row[key]
                with self.subTest(family=r['family'],missing=key),self.assertRaises(ValueError):execute(m,c,row,fake_path())
    def test_unreadable_current_refused(self):
        old=COLLISION['recorded_producer']
        for r,m,c in self.each():
            for name,path in [('stat-permission',fake_path(PermissionError('denied'))),('stat-io',fake_path(OSError('I/O'))),
                    ('boot-missing',fake_path(boot_values=[FileNotFoundError('missing boot')])),
                    ('boot-permission',fake_path(boot_values=[PermissionError('denied')]))]:
                with self.subTest(family=r['family'],case=name),self.assertRaises(ValueError):execute(m,c,copy.deepcopy(old),path)
    def test_ambiguous_current_refused(self):
        old=COLLISION['recorded_producer'];pid=old['pid'];ticks=old['start_ticks']
        for r,m,c in self.each():
            paths=[fake_path(''),fake_path(proc_stat(pid+1,ticks)),fake_path(proc_stat(pid,0)),
                fake_path(proc_stat(pid,-1)),fake_path(proc_stat(pid,'abc')),
                fake_path(proc_stat(pid,ticks),[BOOT,OTHER_BOOT]),fake_path(proc_stat(pid,ticks),['bad'])]
            for index,path in enumerate(paths):
                with self.subTest(family=r['family'],case=index),self.assertRaises(ValueError):execute(m,c,copy.deepcopy(old),path)
    def test_process_disappearance(self):
        for r,m,c in self.each():
            with self.subTest(family=r['family']):execute(m,c,copy.deepcopy(COLLISION['recorded_producer']),fake_path(ProcessLookupError('process exited')))
    def test_historical_static_ownership_still_refused(self):
        bad={'mode':'compress','argv':['wrong'],'complete':False,'exit_code':1,'absent':False,
             'cpu_affinity':[],'observed_cpu_affinity':[],'launched_executable_sha256':'0'*64}
        for r,m,c in self.each():
            for key,value in bad.items():
                row=copy.deepcopy(COLLISION['recorded_producer']);row[key]=value
                with self.subTest(family=r['family'],key=key),self.assertRaisesRegex(ValueError,'producer codec ownership/exit'):
                    execute(m,c,row,fake_path())
    def test_exact_observed_self_collision(self):
        old=COLLISION['recorded_producer'];new=COLLISION['actual_reader']
        self.assertEqual(old['pid'],new['pid']);self.assertEqual(old['boot_id'],new['boot_id']);self.assertNotEqual(old['start_ticks'],new['start_ticks'])
        for r,m,c in self.each():
            with self.subTest(family=r['family']):execute(m,c,copy.deepcopy(old),fake_path(proc_stat(new['pid'],new['start_ticks'])))
    def test_exact_source_inverse(self):
        helper=(P/'identity-helper.txt').read_text()
        for r,m,c in self.each():
            before=Path(r['preserved_source_copy']['path']).read_text();after=Path(r['corrected_reader']['path']).read_text()
            with self.subTest(family=r['family']):
                self.assertEqual(after.count(helper),1)
                self.assertEqual(after.replace(helper+'\n','',1).replace(B['new_atom'],B['old_atom']),before)
                self.assertEqual(Path(r['prior_readback_reader']['path']).read_bytes(),before.encode())

def source_inputs():
    paths=[Path(__file__),P/'identity-helper.txt',P/'reader-bindings.json',P/'collision.json']
    for r in B['rows']:
        paths.extend(Path(r[k]['path'])for k in ('prior_readback_reader','preserved_source_copy','corrected_reader'))
    return {str(p):dict(bytes=len(p.read_bytes()),sha256=hashlib.sha256(p.read_bytes()).hexdigest())for p in paths}

if __name__=='__main__':
    ap=argparse.ArgumentParser();ap.add_argument('--output',required=True);a=ap.parse_args();out=Path(a.output);out.mkdir()
    before=source_inputs();started=time.time_ns();stream=io.StringIO()
    result=unittest.TextTestRunner(stream=stream,verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(IdentityControls))
    after=source_inputs();text=stream.getvalue();(out/'stderr.txt').write_text(text);sys.stderr.write(text)
    doc=dict(complete=result.wasSuccessful()and before==after,tests_run=result.testsRun,reader_families=len(READERS),
        failures=len(result.failures),errors=len(result.errors),source_inputs_unchanged=before==after,
        source_inputs=before,started_ns=started,ended_ns=time.time_ns(),argv=sys.argv,
        scope='Mocked /proc identity and the actual AST-extracted ownership check only. No reader verify(), payload, decoder, workload, or historical acceptance rerun.')
    (out/'result.json').write_text(json.dumps(doc,indent=2,sort_keys=True)+'\n');print(json.dumps({k:v for k,v in doc.items()if k!='source_inputs'}))
    sys.exit(0 if doc['complete']else 1)
