# Public RPC framework experiment

Tracking: #9, #13 and #20. Compare unary tonic/HTTP2, tarpc 0.38/TCP and a
[bounded streaming-gRPC control](RPC-STREAM-CONTROL.md) for client-visible RawKV.
These opt-in prototypes are based on selected runtime `23bc58b`; neither alternate
transport is a production selection. The exact b49 unary/tarpc
[local diagnostic](https://github.com/c4pt0r/kv9/blob/aa56fac/scripts/redis-reference/results/b49a2f6-rpc-framework-c64-tmpfs-diagnostic.md)
records +48.07% GET, +26.31% PUT and +32.98% mixed throughput, with its explicit
volatile single-host scope. Streaming measurements remain open. Raft consistency
is mandatory.

## Experimental boundary

Compile the explicit `rpc-experiment` feature. Normal builds contain no alternate
listener. Each voter requires a nonzero loopback `KV9_RPC_EXPERIMENT_ADDR` to
enable tarpc. Streaming uses its separate `KV9_GRPC_STREAM_EXPERIMENT_ADDR`.
Administrative and inter-voter Raft traffic keep the existing gRPC endpoints.
The workload configuration records `rpc_transport` as `tonic_unary`, `tarpc_tcp`
or `tonic_stream`; comparison arms use the same feature-enabled
server/client artifacts. Reports bind the transport setting to Cargo feature
records and retained executable/source hashes.

Tarpc multiplexes point GET, PUT and DELETE over persistent TCP with NODELAY.
The outer envelope uses bincode; inner requests/responses retain the same prost
protobuf payloads. This isolates framework/framing differences without also
claiming an optimized native-bincode API. Request and response Debug rendering
redacts authentication, values, metadata and status prose.

The adapter authenticates through the same TokenAuthenticator and calls the
same Kv9Grpc point methods directly in process. Both public listeners share the
same admission ledger and backend. There is no internal public-gRPC forwarding
hop. Raft transport, engine/WAL, sync calls and read/write receipt logic are
unchanged by this experiment.

## Conditional transport refinement argument

Assume the selected authenticated handler/backend implements its documented
Raft read/write contract and the framework delivers only decoded, correlated
responses for a call. The adapter must preserve that contract as follows:

1. Authentication constructs the same trusted request extension before any
   point handler runs. A malformed/authentication-refused request executes no
   handler and therefore no backend effect. Point protobuf decoding produces
   exactly the context/key/value sent by the existing client encoder.
2. Each received point call invokes at most one matching existing handler.
   The adapter introduces no mutation buffer, batching rule or retry. A
   successful write reply is constructed only after that handler returns its
   exact committed/applied position. A successful GET comes only from the
   existing quorum-confirmed read path. Thus projection onto handler invocations
   preserves their operations and their successful-response order.
3. ASCII and binary metadata entries, including duplicates, status codes,
   details and payloads are transported without collapsing conflicting control
   fields. Existing client classification rejects ambiguous markers. Invalid
   response codes/shapes/metadata become DataLoss without a trusted refusal
   marker. A lost response, broken channel or RPC failure becomes an unconfirmed
   transport status, never a success or a fabricated precommit refusal.
4. The existing client retains its one absolute monotonic deadline, bounded
   attempts and capacity permit. Only explicit valid NotLeader refusals enter
   its retry edge. The failed write's logical call terminates as unknown on
   transport uncertainty. Reconnection is permitted only for a later call:
   cached channels are generation-owned, finished dispatch tasks are replaced
   under one initialization lock, and an old call cannot invalidate a newer
   channel or replay its mutation into that channel.
5. Dropping a handler future does not release ownership of a live prepared
   write: the existing handler transfers completion and its admission
   reservation to its owned task. The transport neither releases that
   reservation nor acknowledges the unfinished write. Subsequent admission
   still observes its occupancy until completion.

By induction on adapter call/reply events, every successful point response maps
to an existing handler success, and every extra failure maps to a refusal or
uncertain result without introducing an additional successful effect. This is a
conditional adapter argument, not a machine-checked proof of tarpc, serialization
libraries or the complete Rust/Raft runtime. The source-mapped core proofs and
their remaining composition obligations retain their separate scope.

## Bounds and connection lifecycle

Both directions cap framed bytes at 1 MiB plus 8 KiB envelope allowance; point
payloads retain the 1 MiB limit. The server accepts at most eight active channels
and at most 256 concurrent framework requests per channel. Public handler
admission retains its shared request/encoded-byte limits. These are distinct
bounds: public admission does not account for all decoder/framework allocations.

Client pending and in-flight request limits use the existing configured limit,
at most 256. Each endpoint caches one active channel generation and serializes
connection creation. A generation owns its dispatch task; dropping it aborts
that task. A server listener owns its connection JoinSet, so dropping the
listener aborts outstanding connection tasks. With the retained default unwind
builds, connection failures, including a
malformed deadline that panics the pinned framework decoder, are isolated from
the listener and voter. No application payload is logged by that containment.

Client deadlines use Tokio/std monotonic Instants. The framework serializes a
remaining duration, so server deadline reconstruction is approximate across
transit; the existing outer client deadline remains authoritative. Cancellation
does not prove a mutation did not apply.

## Reproduction and acceptance

Run locally, outside any timing benchmark or full Chaos load:

```sh
python3 scripts/build-rpc-experiment.py --output /tmp/kv9-rpc-build
python3 scripts/rpc-experiment-e2e.py --build /tmp/kv9-rpc-build --output /tmp/kv9-rpc-e2e
```

The builder retains separate standalone server and workload builds, their
commands/features and same-source manifests. Add `--release` only for a later
measurement build. The real-process fixture uses ordinary WAL storage, both
transports, complete independently checked histories, leader termination and
original-directory restart. It checks successful measured GET/PUT invocations
fully contained after kill and after restart, executable/writer identity and
public/read/apply drain. SIGKILL is not physical power loss or Chaos Mesh.

Unit transport controls exercise malformed/authentication replies, control
metadata ambiguity, real TCP dispatch, cancelled/deadline writes, connection
replacement and malformed-connection containment. Before any production
promotion, retain actual Chaos Mesh histories and protocol/refinement evidence.
Compare QPS and latency only after functional checks, under repeated identical
workloads, payloads, CPU masks, admission and outcome accounting. Do not
attribute earlier a00e benchmark results to this feature build.

No Redis-class result is claimed. Finish that performance target before dynamic
multi-Raft and automatic range splits. DPDK remains conditional on measured
network cost; a loopback RPC comparison does not establish a NIC bottleneck.

## Original b49 local validation, 2026-09-10

The opt-in workspace run passed 653 tests/doctests, zero failed and 23 ignored.
Two subsequent regressions extended the focused adapter suite to ten passing
tests, including channel replacement without replay and malformed-deadline
containment. Final all-target workspace Clippy with warnings denied passed.
The default-feature workspace separately passed 645 tests/doctests, zero failed
and 23 ignored. Six independent schema/build-feature corruption controls were
rejected as intended. These counts do not claim a single 655-test execution.

The second three-voter fixture passed independent full-history checks:

| Transport | Complete operations | Successful | Unknown |
| --- | ---: | ---: | ---: |
| Unary tonic | 143 | 131 | 12 |
| Tarpc/TCP | 149 | 136 | 13 |

Both histories include successful GET/PUT invocations fully contained during
leader loss and after original-directory restart. Unknown writes remain
terminal; only typed NotLeader prefixes enter the existing retry edge. All
six node/case drain observations use a same-lifetime baseline captured after
client exit, then two separately observed export advances before accepting zero
public/read/apply occupancy and stopped=false. Five voter and two client
lifetimes exited. This is an ordinary-WAL functional run with concurrent local
test activity, not a QPS measurement or a Chaos Mesh run.

The retained first and second builds execute identical server SHA-256
`1eb8ea9f86d51bfca066d2c51292188c93af218ad1e2425c2deacf0e7ce67b69`
and client SHA-256
`9e5b26ada6d31d0f9979798d71f0be18471b24a64f888ec1c63cadb34b999d13`.
Engine/Raft feature lists are empty; root/server/client explicitly enable
`rpc-experiment`. Build manifests truthfully retain a dirty source tree based
on `23bc58b`; all 421 second-build inputs were independently reopened before
this documentation closeout. The only change between those two build source
inventories is the E2E freshness repair, with no runtime change.

The first fixture passed histories and progress, but did not retain a fresh
post-workload status-export boundary for its zero-ledger observation. Its old
metrics timestamps do not prove the status itself was stale or that queues
remained occupied; the missing freshness attestation precludes accepting that
drain claim. Keep this run and its scoped audit unchanged. The second fixture
adds the serial observations and reruns the local scenario. Earlier compile
and test-fixture lint failures are also retained; no acceptance check was waived.

Evidence paths:

- `/tmp/kv9-rpc-framework-build-{first,second}`: commands, feature records,
  source inventories and separately retained server/client executables.
- `/tmp/kv9-rpc-framework-e2e-{first,second}`: histories, reports, writer-bound
  status, fault windows, process identities and cleanup.
- `/tmp/kv9-rpc-framework-independent-audit/second/audit.json`: accepted audit,
  SHA-256 `84c4c2343c9304e549344f8a7f28863c7eacd137cc437e3961d0d26c79370a14`.
- `/tmp/kv9-rpc-framework-independent-audit/audit.json`: first scoped audit,
  SHA-256 `f900790b6858a75a971e4504c8788568fbbfa9a2659f2346cf3e1d50d7db29b3`.
- `/tmp/kv9-rpc-framework-workspace-first.log`,
  `/tmp/kv9-rpc-framework-default-workspace.log`,
  `/tmp/kv9-rpc-framework-regressions-first.log` and
  `/tmp/kv9-rpc-framework-clippy-final.log`: exact local test/lint outcomes.
- `/tmp/kv9-rpc-framework-report-controls/result.json`: six rejected
  transport/schema/feature corruptions against the accepted report validator.

The experiment remains separate from main. The subsequent exact-b49 repeated
release measurements are linked above; they do not establish the newer streaming
candidate's performance. Streaming correctness/measurement evidence and exact
experimental Chaos acceptance have their own gates. Earlier default-runtime
Chaos evidence is not reused as feature acceptance. No hosted workflow was
dispatched.
