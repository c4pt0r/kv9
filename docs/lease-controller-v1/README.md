# Lease controller source and arithmetic validation

See the [implementation contract and remaining adapter work](../LEASE-CONTROLLER.md).
This is an experimental Rust algorithm component. It does not enable a server
lease read, mint a production `ReadBarrier`, qualify a physical clock or establish
a performance result. Selected runtime remains `11113f6` / Safe ReadIndex.

The local source/cache transaction completed with exit 0:

- Default Raft library compilation and **205 tests passed**, zero failed/ignored,
  including **25 new lease tests**.
- Explicit `experimental-leader-lease` compilation and all-target Clippy with
  warnings denied passed, along with formatting.
- The shared `BuildCache` lock and first-party invalidation were used. Complete
  source inventories before/after the transaction agree. The original test
  executable, Cargo artifacts and command records remain source-bound.
- The existing disk guard retained an 80-GiB free floor plus a 16-GiB initial
  reservation; its five-second samples account conservatively for all filesystem
  writers. No build budget was increased.

The [source summary](source-summary.json) records commands and the retained test
binary digest. This is a Raft library gate, not a complete workspace release,
server recovery or actual Chaos campaign. Subsequent additions to proof scripts
and documentation did not alter any tested Rust/Cargo input.

The separate optimized component/proof runner also completed with exit 0:

- The actual module and its 25 tests pass with `rustc -C opt-level=3` before and
  after the controls. This is an optimized component check, not a new server build.
- Eight mutated source variants fail their intended tests: missing quarantine,
  early voting, publication without quorum, mixed-round ACKs, stale frontier,
  early view, missing generation validation and extended ticket expiration.
- Deriving Clone for the read ticket fails with `E0080` at the intended guard.
- Three integer SMT checks prove floor division, ceiling division and their
  constructor composition, including `u128` intermediate bounds. Four incorrect
  arithmetic variants produce satisfying countermodels. Baseline/restored
  sources match. See [controls-summary.json](controls-summary.json).

These test populations overlap. Source fault controls are intended failed tests,
not generated distributed linearizability histories. In particular, final
ticket-expiration rejection is a conservative protocol rule; an overlapping
paused read is not automatically a stale-read counterexample.

The first monolithic arithmetic query timed out at five seconds. Its source,
command and actual `timeout` result remain included. The completed proof first
proves generic floor/ceiling division facts, then instantiates them in the
constructor query. This does not increase the solver time bound or assume the
desired lease inequalities. The input ranges establish each dependency's
nonnegative-numerator and positive-denominator premises.

The [original evidence archive](original-evidence.tar.gz) contains **253 files,
463,238 bytes**, SHA-256
`44d891a17265c0356566bae546586108f268b2f8e5cc31780d0071b55f86d943`.
Every member was independently read back and compared; [inventory.json](inventory.json)
records exact names, lengths and hashes. Compiled test executables and the large
cold TLC state payloads are retained locally rather than included in this archive.
Original sources, Cargo/cache logs, test outputs, proof variants/countermodels
and source/storage readback records are included.

The archive also retains the delegated test-environment maintenance metadata:
2,043 files from three terminal TLC state stores were archived, member-verified
and rehashed in place before removing only those originals. Cold archives total
976,242,766 bytes; observed free space increased by 2,411,061,248 bytes. Original
model/config/log/result files and their incomplete verdicts were unchanged.
This storage work is not additional model-check or Chaos acceptance.

Reproduce optimized controls and integer proofs in a new output directory:

```sh
taskset -c 6-15,22-31 env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 \
  python3 scripts/check-lease-controller.py \
  --z3 /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/lib/tlapm/backends/bin/z3 \
  --output /tmp/kv9-lease-controller-controls-new
```

The original supervised Cargo commands and cache helper are in the archive.
They check `cargo test --locked -p kv9-raft --lib`, the explicit experimental
feature and Clippy, with one root-owned Cargo transaction. Reproduction must
use fresh output paths, the same source-build disk guard and cache ownership;
no performance timing may overlap builds or validation. The solver is pinned
Z3 4.8.9 with a five-second bound per query. No hosted CI ran for this checkpoint.
