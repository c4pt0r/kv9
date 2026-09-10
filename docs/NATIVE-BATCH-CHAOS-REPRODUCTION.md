# Reproducing additive native-batch Chaos acceptance

The opt-in repository fixture runs the original 21 actual Chaos Mesh windows,
CLI/catalog workload and persistent point workload, and adds a separate native
atomic-batch client. Normal `scripts/chaos-mesh-e2e.sh` invocation keeps its
existing point-only build and workload behavior. Set the native plan only via
the prepared runner described below.

This is correctness testing on a local kind cluster. It makes no latency,
throughput, cross-host, power-loss, dedicated client-link or quorum-loss claim.
The native client uses normal default-feature `tonic_stream` on the same
advertised gRPC port, 20160; there is no alternate listener, relay or experimental
transport feature. Native operations still use Raft's existing consistency path.

## Files and evidence contract

- `scripts/native_batch_chaos/prepare.py` builds and retains the normal server,
  pressure example, point workload and native workload **separately**, then stages
  and loads `chaos/native-batch.Dockerfile`. The source must be clean; artifacts,
  Cargo target and image context stay outside the repository. Source inventories,
  exact Cargo artifacts/features, binary hashes, image payloads, Docker config ID
  and per-kind-node CRI manifest mappings are retained.
- `freeze.py` records cluster/CRD/controller readiness and namespace UIDs, verifies
  all prepared bytes, and writes immutable `ready-plan.json`. `plan.json` records
  mutable launch/terminal state and the immutable plan's SHA-256. Preparation
  never launches the matrix.
- `run.py` verifies that plan, launches the existing main fixture and its bounded
  exact-runtime observer, and retains original exit codes. It refuses a second
  launch with the same plan. A failed attempt is never reused or relabeled.
- `fixture.sh` adds a distinct native keyspace/client, phase mirrors, bounded
  history gates, full final history collection and fresh final replica drains.
  It also makes the previously separately captured point-client executable,
  process/configuration and ordinary-port socket evidence part of baseline setup.
  `scripts/chaos-mesh-persistent.sh` is unchanged.
- `native-gate.py` requires new fully completed successful BatchGet **and**
  BatchPut calls after a serial complete-history-prefix barrier. Its entire
  observation deadline is 25 seconds. Raw commands, before/after Pod/container
  identities, PID/start/boot/executable/configuration and prefixes are retained.
  Native progress has no point-workload stage or clock field: none is invented.
- `check-native-windows.py` replays the complete atomic history and confirms that
  each selected call's **invocation and return** fall inside the actual fault
  envelope. Fault UID, action and entire spec must agree; injected continuous
  faults cannot already be recovered. Instant owner-death cases additionally
  require the original physical-refusal or endpoint brackets.
- `native-final-drain.py` requires all four final replica lifetimes to publish
  two serial fresh, empty, nonfatal `Serving` states within 20 seconds, after both
  client collectors have been removed. The original point delay observation gate
  separately preserves its 20-second freshness and exact-process requirements.
- `readback.py` reruns the original history/effect validators and the maintained
  native/observer checks **on a new copy**, rehashes the original input inventory,
  and preserves the original harness result in its own summary. An optional
  retained cleanup directory also binds all observed server lifetimes to their
  container exit/absence evidence. It performs no cluster operations.

The explicit native configuration is fixed: 4 workers/in-flight calls, 4 keys,
8 items per batch, 128-byte values, seed 40, operation mix `[10,10,10,35,35]`
(get/put/delete/BatchGet/BatchPut), at most 6,000 traffic calls, 30 minutes,
500-ms progress exports and 256 MiB full-history bound. Attempts remain 6,
request deadline 1,500 ms and retry backoff 10 ms. A cap or timeout fails the
matrix; it is not extended to make a run pass. All unknown and refused outcomes
remain in the complete history. Initialization/final verification may use smaller
batches. The original point configuration and all CLI predicates remain intact.

## Prepare a new source-bound run

Use an already provisioned local kind cluster with the project's accepted Chaos
Mesh setup, working Docker/kubectl/kind tools, the PodChaos/NetworkChaos/IOChaos
CRDs, ready Chaos controllers/daemons and sufficient disk for retained debug
binaries, images and complete histories. The I/O fixture still uses the existing
`no-statx` wrapper and its existing local PVC paths. Do not change fault selectors,
retry policy or history bounds to repair environment problems.

The following variables are declared inputs, not baked-in host paths. Choose a
new output directory, new private Cargo target and a unique local image tag for
every attempt. Run from a **clean committed checkout containing these helpers**.
Preparation builds binaries and an image and loads that image into the specified
kind cluster; it does not inject faults or remove historical namespaces.

```bash
export KV9_CHAOS_WORK=/path/to/new-evidence-directory
export KV9_CHAOS_TARGET=/path/to/new-private-cargo-target
export KV9_CHAOS_KUBECONFIG=/path/to/local-kind.kubeconfig
export KV9_CHAOS_KIND=/path/to/kind
export KV9_CHAOS_CLUSTER=your-local-kind-cluster
export KV9_CHAOS_TAG=kv9-chaos:native-batch-unique-attempt

PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/prepare.py \
  --output "$KV9_CHAOS_WORK" --target "$KV9_CHAOS_TARGET" \
  --kubeconfig "$KV9_CHAOS_KUBECONFIG" --kind "$KV9_CHAOS_KIND" \
  --kind-cluster "$KV9_CHAOS_CLUSTER" --image "$KV9_CHAOS_TAG"

sha256sum "$KV9_CHAOS_WORK/ready-plan.json"
```

Every Python helper rejects optimization mode before imports or any fixture action, because assertions are part of the acceptance contract. Keep `PYTHONOPTIMIZE=0` and do not pass `-O` or `-OO`; command-line optimization flags override that environment setting.

Keep the frozen inputs unchanged. After establishing the local CPU quiet window
and reviewing the exact ready plan, set only the mutable launch flag and run:

```bash
PYTHONOPTIMIZE=0 python3 - "$KV9_CHAOS_WORK/plan.json" <<'PY'
import hashlib, json, sys
from pathlib import Path
path = Path(sys.argv[1])
p = json.loads(path.read_text())
assert p['plan_ready'] and not p['fixture_started']
assert hashlib.sha256(Path(p['ready_plan_path']).read_bytes()).hexdigest() == p['ready_plan_sha256']
p['fixture_launch_authorized'] = True
path.write_text(json.dumps(p, indent=2) + '\n')
PY
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/run.py --plan "$KV9_CHAOS_WORK/plan.json"
```

The runner writes `matrix.log`, `observer.log`, exact child PIDs and terminal
results to the declared preparation directory. The raw run appears under its
`runs/kv9-chaos-e2e.*` directory and is recorded in `plan.json`. Monitor the same
process until terminal; an observation timeout is not a reason to restart it.
Host tooling runs on CPUs 6–31. Pod CPU masks are observed separately; this is a
shared-host correctness fixture, not a core-isolated performance experiment.

The existing main fixture preserves the namespace on failure and does scoped
best-effort cleanup on success. A successful harness exit does **not** by itself
prove namespace/process cleanup. Retain and independently check namespace UID,
all observed server container exits, collector exits and historical namespace
UIDs before a cleanup/archival closeout. Never remove historical namespaces to
make readiness pass. The source runner does not automatically authorize further
faults or cleanup after a failed matrix.

## Read back without running faults

Supply the actual retained plan and artifact, and a fresh output directory:

```bash
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/readback.py \
  --plan "$KV9_CHAOS_WORK/plan.json" \
  --artifact /path/to/retained/kv9-chaos-e2e.actual \
  --output /path/to/new-readback-output
```

For the accepted historical run, also supply `--cleanup` pointing to the retained
cleanup directory containing `summary.json` and
`all-server-lifetimes-exited.json`. Existing validators must match their original
source hashes. This requires the retained source and preparation paths referenced
by the original plan; relocation must use an explicit audited path mapping, not
rewrite an accepted plan and claim its original hash. No live cluster, image build
or fault injection is needed for readback.

The corrected parser accepts dot or comma fractions with 1–9 digits, explicit
UTC/offset timestamps, and preserves integer nanoseconds. Its original GNU-date
input is retained in `timestamp-original.txt`. Run all 14 parser controls, the
24 configuration/drain controls, and 18 baseline/mutant/restored window controls:

```bash
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/timestamp-controls.py \
  --timestamp scripts/native_batch_chaos/timestamp-original.txt \
  --output /path/to/new-timestamp-controls
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/config-drain-controls.py \
  --output /path/to/new-config-drain-controls
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/window-controls.py \
  --plan "$KV9_CHAOS_WORK/plan.json" \
  --artifact /path/to/retained/kv9-chaos-e2e.actual \
  --output /path/to/new-window-controls
```

Check the optimized-mode refusal without any build, cluster call or history replay:

```bash
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-31 python3 \
  scripts/native_batch_chaos/optimization-controls.py \
  --output /path/to/new-optimization-controls
```

This checks every Python file with inherited `PYTHONOPTIMIZE=1`, explicit `-O`
and explicit `-OO`, plus normal-mode `--help`. An audit hook rejects filesystem
writes, subprocess launches and network operations before they occur. Three
retained copies with the guard removed demonstrate that the hook catches unsafe
prepare/run/readback behavior. The protected inputs and prospective output paths
must remain unchanged throughout. The 24-file suite passed 72 optimized refusals,
24 normal-mode checks and 3 guard-removal traps. Its first wrapper attempt lacked
the direct-script module search path; that control-only failure was retained and
the corrected second attempt passed. No historical matrix or readback was rerun
for this guard.

Window controls use synthetic process/clock samples and retained resource shapes;
they test rejection behavior and all 21 original fault predicates. They are not
new actual Chaos results. Each output directory is exclusive and every failed
attempt remains retained.

## Relationship to the executed overlay

The executed source was clean `5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a`.
Its original matrix exited **1** because the final checker rejected the valid GNU
date timestamp `2026-09-10T13:34:44,807368542-07:00`. The separately corrected
same-artifact audit accepted all effects/histories/drains. That failure, original
checker and original raw `native-window-audit.json` remain unchanged.

This integration starts at `d31fc448eeb2a41744091af4654c5e35e5a43dd1` and changes
only fixture packaging/tooling and this document. Differences from the executed
overlay are explicit:

1. Repository-relative imports and declared preparation/evidence paths replace
   frozen host paths. Container `/tmp/workload*`, wrapper PID files and `/data`
   paths remain intentional fixture protocol paths.
2. The existing main driver gains conditional additive hooks. Its point-only
   build branch remains available; its persistent script is unchanged. The
   prebuilt native branch copies retained evidence rather than staging source
   `target/` files. Native catalog additions remain part of initial CLI state.
3. The separate Dockerfile is byte-identical to the executed native image recipe.
   Preparation and readiness are maintained command-line entrypoints, including
   failure logs and source/Cargo/image/namespace bindings.
4. A `__debug__` guard is now the first executable statement after each Python module docstring, before imports. It refuses disabled assertions. Apart from this guard and path/entrypoint adaptation, the corrected dot/comma timestamp parser replaces the rejected original one.
   Its predicates, native capture/gate/drain, original process probe, observer
   field checks and delay-gate declarations match the retained corrected or
   executed source. `correspondence.py` records this comparison, both hashes and
   the main-driver diffs. The checker adds an optional separate output path.
5. The point-client identity capture previously run alongside the matrix is now
   called in baseline setup after the original point client is ready. It adds no
   work inside a fault window and does not change either workload.
6. The independent inline/apply/delay readbacks now take explicit plan/artifact
   arguments. `readback.py` copies inputs and combines the unchanged original
   validators with those maintained checks; original raw files remain untouched.

The maintained source passed syntax checks, the 14/24/18 controls and source
correspondence checks. The first new window-control invocation exposed a Python
CLI-argument/local-variable name collision; its failure log was retained, the
control adapter was fixed, and the new attempt passed. Maintained same-artifact
readback passed, then passed again after adding raw point-process/socket and
all-server-lifetime cleanup comparisons:

| History | Calls | Successful | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,815 | 5,225 | 574 | 16 |
| Persistent point | 1,542 | 1,521 | 15 | 6 |
| Native mixed atomic batch/point | 2,716 | 2,669 | 32 | 15 |

Native history contains 953 BatchGet calls / 7,618 ordered items and 950 BatchPut
calls / 7,597 ordered items, including initialization/final verification. All
21 windows plus native baseline, 1,404 exact-runtime samples across 429 observer
batches, 31 server lifetimes and all four fresh final drains remain accepted.
The original harness exit remains 1 in every new readback summary.

The new prepare/run orchestration has been reviewed and syntax-checked; **no
build, matrix rerun, cluster mutation or performance run was performed to validate
this integration**. A future clean-source run will generate its own independent
runtime/feature/image identity and cannot inherit the historical run's exit code
or acceptance claim. Dedicated native client-link, same-process reset and
quorum-loss scenarios remain a separate fixture and acceptance set.
