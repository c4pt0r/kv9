# ThinLTO release code generation experiment

This candidate starts from `40f014f`, whose default runtime is selected CRC
`ca0002c7` with optional read-stage observation disabled. It changes only release
code generation: ThinLTO and one codegen unit. Runtime/algorithm source, locked
dependencies, the target CPU baseline, panic unwinding, overflow behavior and
all feature defaults are unchanged. No host-specific ISA option is enabled.

Earlier scheduling experiments already rejected fixed global queue intervals
and two persistent request workers. Revisiting those designs without a new
measured cause repeats existing work. The current CPU observations also show
substantial RPC/framing and allocation work across crate boundaries. ThinLTO
allows cross-crate optimization and one codegen unit reduces within-crate
optimization boundaries. This is a build hypothesis, not a predicted speedup.
Tradeoffs include greater build time/memory and potentially different code size
and instruction-cache behavior. Retain measured throughput and latency together.

Root runs local release workspace correctness checks and all-target Clippy,
then retains a clean source-bound server and workload under the shared cache
lock with first-party invalidation. The changed profile may also rebuild locked
dependencies; bounded disk observations, build time and the original first failure
must be retained. Both correctness and production artifacts use this profile.

Ordinary leader-loss/restart histories precede the complete opposite-order
c1/c64 GET/mixed screen against original CRC and Redis. The timed native/Redis
clients remain the original fixed v3 artifacts: this measures server code
generation without also optimizing the measurement client. Default features
exclude diagnostic/testing seams. No correctness predicate, storage guard,
workload, success accounting or percentile definition is changed.

No Raft, metadata, engine, admission, cancellation, deadline or receipt transition
changes. Fresh Safe ReadIndex, sealed groups, successful whole-pump completion,
unified apply/view fences and durable acknowledgements retain their existing
implementation and proof scope. Existing proofs assume a correct compiler;
this experiment is not a compiler or whole-implementation proof. A favorable
screen remains experimental until broader API and actual exact-build Chaos
acceptance. Shared-host tmpfs-WAL comparisons do not establish real-disk,
cross-host, sustained capacity, equal Redis durability or industrial readiness.
CI remains local and no hosted workflow is dispatched.
