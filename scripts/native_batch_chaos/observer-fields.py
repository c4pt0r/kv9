"""Additional owned status observer; no production/runtime instrumentation."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
def validate(parsed,pod,defaults):
 env={v['name']:v.get('value') for c in pod['spec']['containers'] for v in c.get('env',[])}
 expected=dict(public_rpc_limit_requests=env.get('KV9_PUBLIC_MAX_REQUESTS',str(defaults['public_rpc_limit_requests'])),public_rpc_limit_encoded_bytes=env.get('KV9_PUBLIC_MAX_ENCODED_BYTES',str(defaults['public_rpc_limit_encoded_bytes'])),raft_async_read_limit=str(defaults['raft_async_read_limit']),raft_async_apply_limit=str(defaults['raft_async_apply_limit']))
 states={}
 for when,key in [('before','state'),('after','state_after')]:
  state=parsed[key]
  for name,value in expected.items():assert state[name]==value,('limit mismatch',when,name,state.get(name),value)
  values={name:int(state['raft_async_apply_'+name]) for name in ['limit','queued','in_flight','peak']}
  assert state['raft_async_apply_stopped'] in ['true','false'],('invalid stopped flag',when)
  assert 0<=values['queued']<=values['in_flight']<=values['peak']<=values['limit'],('async apply occupancy bound',when,values)
  values['stopped']=state['raft_async_apply_stopped'];states[when]=values
 assert states['after']['peak']>=states['before']['peak'],'async apply peak decreased within one process sample'
 return dict(expected_limits_from_pod_and_source=expected,async_apply_observations=states,status_contract_valid=True)

def final_drained(state):
 assert state['raft_async_apply_limit']=='128' and state['raft_async_apply_stopped']=='false'
 assert state['raft_async_apply_queued']==state['raft_async_apply_in_flight']=='0'
 assert 0<=int(state['raft_async_apply_peak'])<=128
