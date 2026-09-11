# Parallel stream requests: local acceptance

Candidate `f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02` replaces the ordinary
stream owner's inline `FuturesUnordered` polling with a bounded `JoinSet`.
Independent handler tasks can execute synchronous preparation and response
encoding on different runtime workers. Each handler still owns its original
response-channel reservation. The stream semaphore grant is shared with all
handlers until actual task cleanup, including cooperative cancellation.

The [source and ownership argument](https://github.com/c4pt0r/kv9/blob/f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02/docs/PARALLEL-STREAM-REQUESTS.md)
establishes the reservation invariant `P + A + B <= CHANNEL_LIMIT` and the
independent task-count bound. This is a source-level scheduling correspondence,
not a new machine-checked proof of the complete runtime. Authentication,
framing, deadlines, response correlation, Raft barriers, metadata validation
and committed/applied write receipts are unchanged. Broader adapter proof
obligations remain open.

The candidate is committed and pushed on `codex/parallel-stream-requests`.
Master remains unchanged. Extra task scheduling and contention can offset the
parallelism; correctness acceptance alone does not establish a performance gain.

## Local gates at the frozen source

| Gate | Result |
|---|---|
| Default workspace | 707 passed, 0 failed, 23 ignored |
| RPC experiment workspace | 717 passed, 0 failed, 23 ignored |
| Both all-target Clippy configurations | Passed with warnings denied |
| Focused real HTTP/2 stream tests | 23 passed |
| Compiled stream-slot control | Baseline passed, early-release mutant rejected, restored source passed |
| Process kill/restart E2E | 360 calls: 331 OK, 29 unknown; both complete atomic histories valid |
| Six-arm comparison correctness smoke | 2,275,453 successful calls; all 20 owned process lifetimes exited |
| Actual Chaos Mesh fixture | All 11 windows passed; full audit and required netem-leaf readback passed |

The new cancellation test closes an actual SDK stream generation with a
30-second request deadline and an independent five-second observation bound.
It holds a read future's destructor: a new stream remains refused until cleanup
releases the old grant. A compiled mutation that drops the handler's grant
early fails at the specific premature-slot-reuse assertion. The panic test
closes the generation, cancels sibling reads, and verifies that an independently
prepared write retains admission until settlement without replay.

Two superseded test-fixture attempts remain preserved. They tried dropping a
raw tonic response while retaining the request sender; the observed server
stream did not close promptly. The final test therefore makes no response-only
half-close claim. The initial failed workspace run used that superseded test.
The final test support avoids panicking during teardown. A handler join failure
terminates the generation when observed; no claim is made that every concurrent
reply is suppressed at the exact instant another handler panics.

Process E2E uses ordinary stream and unary paths with leader termination and
restart from the original data directory. Stream history has 176 calls,
159 OK and 17 unknown. Unary history has 184 calls, 172 OK and 12 unknown;
its preliminary zero-unknown-effect search was invalid, while the retained
whole-unknown-effect witness is valid. Unknown outcomes remain unknown. Both
fixtures retain all source and release bindings and report no cleanup errors.
The process fixture explicitly sets a 16 MiB public byte limit. Source defaults
are 64 requests and 64 MiB; ignored workspace tests are not counted as passes.

The correctness smoke uses a different server CPU budget from the timed
comparison. Its call counts establish completion only and must not be used as
accepted throughput or latency results.

## Actual Chaos Mesh result

The first exact-image run completed with **2,111 calls: 1,829 OK, 70 unknown
and 212 refused**. The initial zero-unknown-effect history search was
inconclusive. The unchanged guided search found a valid atomic-history witness
allowing whole unknown-write effects. This does not mean every unknown write
took effect; all unknown outcomes remain present and uncertain writes are not
replayed.

The eleven windows cover baseline, Service-VIP delay/heal, actual partial
loss/heal, full client partition/heal, exact socket reset/heal, and quorum
loss/heal. Delay, loss and partitions use actual Chaos Mesh. The exact-tuple
reset uses the separately identified non-Chaos Linux `SOCK_DESTROY` helper.

- Configured 250 ms delay produced selected probes of 251,376–251,800 us.
- Actual 30% zero-correlation loss advanced the same netem leaf `5:` / parent
  `1:4` from 2 to 186: **184 drops**, with 107 TCP retransmissions. The required
  separate leaf check excludes the inherited parent-plus-child double count.
- The contained client-partition window had no successful operation of any
  kind, with 13 unknown BatchGet and 14 unknown BatchPut calls.
- The contained quorum-loss window likewise had no successful operation,
  with 48 refused BatchGet and 47 refused BatchPut calls. Voter DROP counters
  advanced by 44, 47 and 42.
- Every positive/healed window completed successful batch reads and writes.
  The exact socket reset retained the same native client process.
- Two serial fresh post-client publications per replica were empty, running,
  Serving and nonfatal. Whole-fixture counters recorded 642 inline batch reads
  and one blocking submission. These include setup and verification; they are
  not per-response or fault-window attribution.

Run session 24098 and full audit session 3590 exited 0. The mandatory leaf
readback also exited 0. All five owned observed container lifetimes exited;
namespace UID `9d55eb8c-1d96-4d4f-9c8e-b4f22ea94610` and both owned node
directories are absent. All eight historical namespace UIDs remain unchanged.
The 50 retained data members, 581 clean source inputs and 134 frozen preparation
files passed the existing readbacks. No runtime or audit rerun occurred.

The Chaos Pods use source defaults without limit overrides. This is correctness
evidence on one shared Kind host with volatile tmpfs; it does not establish disk
or power-loss durability, cross-host availability, the full 21-window matrix,
or the remaining inter-voter partial-loss and storage-stall scope.

## Retained evidence

Release directory: `/tmp/kv9-parallel-stream-release-first`.
The clean default-feature release binds 581 source inputs.

| Artifact | SHA-256 |
|---|---|
| Server executable | `3ed7974e3eebd9dc6a0e91988d3913f2999fe44fa38d989737022481f993d305` |
| Server build manifest | `771e987061e9d27d40bdcebe08a6a85afbbc080446e9684fc05cd24ded3b9c3a` |
| Correctness workload executable | `c5078581529620d46123d871d21bdbfdf3d7d5233f3e857458671ec443b0bb4e` |
| Final workspace and Clippy summary | `366b3b6c180139bb276a6fa31243cb3384edfc3527dbb90e6851c0a4ccd11479` |
| Focused test review | `a74bc3230ccdb54e51d5f56425c73060bb94bbda6ca5648ce69047ddfcbbf9e7` |
| Compiled control summary | `772b6a43659074c1a7fc3041ec09d588d85755d62fb65caab0acfe0d82ff13f3` |
| Process E2E summary | `4297c58ee15392ebe59042a4bfded953df0531ee15eefc9d38edc3d09f89dc83` |
| Correctness smoke matrix | `b106c95b0e4a2e29a6e3876a18c499f59e64648547e0d25b2556c6b6e62ad7ee` |
| Chaos plan | `00ed78090115e58d6130e419f74eeb4ca30bf787399b8cea8829663e246a6b18` |
| Chaos raw summary | `48411fa63ac3b5b2d998a7f12a263f4291919008fc9998234e3ff920bf95009c` |
| Chaos full independent audit | `770c7862af21c3219c7d872b877a704aabb10b66387d9d9e5cc34edb396b7fdf` |
| Required netem-leaf readback | `c696fa328cc563767990a67074c502f81ee05698b36b80328595a5902689e3fb` |
| Chaos complete result | `c4b330c52598ebb3955ac5f13c20f9fb5ddb709e1ed42f826cd6a8fd8538b280` |

Raw local evidence uses the `/tmp/kv9-parallel-stream-` prefix:
`checks-final`, `test-first`, `slot-control-first`, `process-e2e-first`,
and `comparison-smoke-attempt1`. Original failed attempts are retained in
`test-first` and `/tmp/kv9-parallel-stream-workspace-first.log`.

Chaos raw artifacts: `/tmp/kv9-native-link-acceptance-f2c4e85-attempt1`.
Full independent audit, leaf check and compact result:
`/tmp/kv9-native-link-acceptance-f2c4e85-independent`.

The separate [matched performance comparison](PARALLEL-STREAM-GET-PERFORMANCE.md)
supports retaining this scheduling change for continued development. Default
runtime promotion and the broader roadmap gates remain open. No hosted CI was
dispatched.
