# V3 point and batch write correctness smoke

This owned preparation executes twelve correctness cases on the frozen 0be806d
native/Redis clients and the separate accepted 5ee897a server. Exact identities,
configuration, ordering and limitations are in `protocol.json`; `command.json`
contains the complete invocation and required Python environment/CPU mask.

The six pairs are point GET/PUT versus GET/SET (one item), and native batch APIs
versus MGET/MSET (four items), each at 0%, 50% and 100% reads. Both clients declare
version 3. One owned WAL trio serves six fresh keyspaces; each Redis case creates
a fresh independent process on a reserved loopback port. All configured native
peers remain present; the agreed leader is listed first so the healthy smoke can
require exactly one successful SDK attempt for every call, including setup.

The source-bound report validators independently check schema, encoded sizes,
accounting, complete cutoff/drain spans, histogram arithmetic and selected API
labels. The smoke additionally recomputes the warmup mix, checks pure measurement
mixes and positive mixed populations, and requires all-success, one-attempt
populations. Setup and verification retain
batch labels. The final scan/MGET checks all 128 keys and the sentinel, exact
deterministic bytes, configured write nonce bounds and whether that nonce writes
the returned key. These aggregate clients do not retain full call histories;
this is not a concurrent linearizability or atomicity proof. A slot may be
allocated before a second cutoff check declines it, so aggregate issued counts
do not identify an exact contiguous nonce set. The original draft and this
pre-execution review finding are retained in `first-draft-cutoff-correction/`.

The unmodified retained driver is imported only for `source_binding`,
`cargo_server`, `placement`, `affinity` and `fresh_drain`. The unmodified RESP
helper supplies owned subprocess cleanup, process/listener identity, bounded
RESP reads, the independent mixer and deterministic dataset verifier. The
read-only timing trial, observer and zero-nonce validator are never called.
Both helpers are hash-pinned in `run.py` and checked before import. Fixture and
validator imports are included in the frozen client source inventory.

Voter identity/status, owned sockets, CPU placement and resources are retained
before and after each native case, with fresh two-publication drains before,
after the client exits and after the final external scan. Redis identity,
listener, configuration and placement are captured before and after its client.
Every managed client has an executing binary hash plus PID/start/boot binding;
its report start tick and PID must match the captured process. Short bootstrap
and scan CLI invocations retain exact commands, exit codes and raw outputs via
the unchanged fixture; no claim is made that those short commands have separate
live `/proc` identity observations.

The first failed case stops the recipe, retaining its files and original error.
Cleanup only reaps owned children. The existing native helper kills its owned
servers after final observations; Redis cleanup uses bounded TERM/KILL. WAL and
all report files remain under the owned output directory and receive a terminal
hash inventory. No namespaces, containers, historical resources or source files
are modified. Partial execution cannot be labeled complete.

Client `timing_eligible` and raw rates are preserved exactly, including an honest
operation-cap stop if reached. This preparation always declares
`throughput_acceptance=false`; the 500 ms runs are not performance evidence.

The parent released one execution after the clean release completed. Run the
exact `command.json` argv under `env PYTHONOPTIMIZE=0
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31`, capturing an exclusive first log.
No automatic rerun or weakened predicate is authorized.
