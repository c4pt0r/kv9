# Configuration at a committed recovery cut

C04 component increment, 2026-09-15. Tracks
[#14](https://github.com/c4pt0r/kv9/issues/14) under the
[recovery-anchor contract](RECOVERY-RETENTION-CONTRACT.md).

`DiskRaftStorage::configuration_at_committed` now recovers the full membership
effective at an exact committed `(term, index)`. A newer applied membership
cannot substitute for an older state-image cut, even in the same term. A
committed configuration whose apply record is missing makes the lookup
unavailable; it does not certify the preceding configuration.

This supplies one checked input to complete recovery-anchor validation. The
anchor envelope, root/range binding, manifest publication authority, outer
durable retention ledger and atomic snapshot installation remain unfinished.
C04 remains open.

## Implemented contract

The existing protocol log stores an initial full `ConfState` and subsequent
full states paired with their application indices. Open reconstructs an indexed
history from those records and completes the existing recovery durability
protocol before exposing the storage instance. New configuration publications
enter that history only after the existing record write and sync succeed.
The disk format is unchanged.

Lookup holds the protocol writer mutex throughout its observation. It validates
the exact committed cut, finds the greatest applied configuration index at or
before it, validates the selected entry kind and term, then checks that the
remaining prefix contains no unapplied configuration entry. A found view owns a
copy of the full `ConfState`, including outgoing voters, deferred learners and
automatic joint-consensus exit. Later configuration apply cannot mutate it.

| Observation | Result |
| --- | --- |
| Initial membership with no intervening configuration command | Found, with `applied_at = None`; no fabricated `(0, 0)` position |
| Complete indexed history at an exact committed cut | Found, with the selected configuration's exact term/index |
| Committed but unapplied configuration after the selected state | Unavailable, with the first missing configuration index |
| Missing initial record, unindexed/conflicting/decreasing history | Unavailable; no guess using the current membership |
| Compacted protocol prefix | Unavailable when the cut is retained; a missing cut is an error. A certified snapshot base is required before supporting compaction. |
| Failed writer, zero/overflow cut, wrong term, uncommitted/missing cut, wrong selected entry kind or impossible selected term | Error; no view |

Invalid input does not poison a healthy writer. A persistence failure continues
to use the existing writer fence. After restart, an uncertain complete record
may survive and be stabilized, or may be absent; lookup reflects the recovered
record and never grants authority from the failed live writer.

`CommittedConfiguration` has private fields and no caller-controlled constructor
or decoder. This prevents an ordinary descriptor from constructing this view,
but does not bind it to a root, group, destination store or manifest. The next
anchor layer must bind those identities through authoritative providers before
using the view for installation. It is not an authentication or install token.

## Cost and availability boundaries

This is a recovery-only interface. Selection uses the configuration index, but
checking the suffix can scan all retained entries under the writer mutex.
Worst-case time is linear in that suffix. The history cache grows with retained
configuration records. It must not enter online serving or periodic checkpoint
paths; S05 still needs a certified snapshot base and bounded history index.

Ordinary Ready batching, append, commit acknowledgement and quorum rules are
unchanged. Membership publication adds an in-memory history entry. No new
database node or singleton service is introduced: each replica reconstructs the
view from its own durable protocol state. This does not establish cross-host
availability, remote attachment or complete cluster recovery acceptance.

## Proof and implementation correspondence

[ConfigurationCut.tla](../proofs/tla/configuration_cut/ConfigurationCut.tla) and
the [strict proof inventory](../proofs/tlaps/configuration_cut/inventory.json)
contain **11 theorem statements and 95 proved obligations**. The proof is
parameterized by an arbitrary positive finite prefix length, configuration-entry
set and nonzero term function. TLC additionally explores bounded instances.

The retained committed prefix is an explicit Raft/storage premise: committed
entries do not change and applied full configurations come from the existing
ordered membership implementation. This proof establishes selection and
publication ordering; it does not derive a `ConfState` from command bytes or
reprove the upstream membership algorithm. Checksums, protobuf decoding and Rust
execution are tested and source-mapped, not mechanically refined by TLAPS.

| Model state/action | Implementation correspondence |
| --- | --- |
| Fixed prefix, `HCChanges`, `HCTerms` | Committed entry kind/term observations through durable storage; the exact caller cut is checked first |
| `hcDurable`, `HCSync` | Complete configuration records stabilized by successful sync, or by existing reopen recovery |
| `hcApplied`, `HCPublish` | Full configuration history published after persistence under the same writer mutex |
| `HCFail`, `HCRecover` | Failed live writer refuses; recovery can retain or lose an uncertain pending record before rebuilding the history |
| `hcKnown`, `HCAmbiguous` | Missing initial authority or ambiguous/unindexed/conflicting history cannot supply a found result |
| `HCQuery` | Exact term, healthy writer, known history, selected configuration kind/term and missing-configuration suffix checks |
| Captured view fields | Owned result remains a historical observation after later failure or membership apply |
| Query stuttering/fairness | Stable local inspection eventually settles if its enabled lookup is scheduled; no writer-starvation or wall-clock latency guarantee |

The model permits any applied candidate at or before the cut, a superset of
Rust's greatest-index selection. A found result is nevertheless the latest
committed configuration, durable, correctly termed, and sampled from a healthy
writer with known history. `HCCompleteHistorySucceeds` additionally proves
success with complete applied history, greatest-candidate selection and
monotonic prefix terms. The stable-inspection theorem proves a terminal answer,
which can be a refusal; it does not promise a usable anchor from missing data.

## Actual local validation

- Raft library: **257 tests pass**, none ignored. Seven configuration tests
  include historical initial/joint/stable membership, same-term future exclusion,
  committed-but-unapplied refusal, wrong cuts/entry kinds, missing/compacted
  authority, ambiguous replay and failed persistence/reopen.
- The persistence test exercises **20 actual ModelFs cuts**: EIO and ENOSPC
  before/after write and sync, short writes, and both lose/keep-unsynced crashes.
  These are deterministic filesystem faults, not Chaos Mesh runs.
- Raft all-target Clippy with warnings denied, formatting, and the default
  library build pass. A separate testing-feature build supplies the recorded
  dependencies for isolated source controls.
- Five isolated Rust mutations compile and fail their exact selected test:
  remove the cut bound, skip the missing-configuration scan, ignore the writer
  fence, ignore the caller term, and ignore ambiguous history. An unchanged
  source copy first passes the seven configuration tests; these overlap the
  257-test suite and are not additional test coverage.
- The reproducible formal runner checks strict/no-fingerprint proof execution,
  semantic assumptions, theorem inventory, standard modules and source hashes;
  three positive models, eight model counterexamples and three proof rejection
  controls are recorded in the [portable evidence](configuration-at-cut-v1/README.md).

| Positive TLC instance | Generated states | Distinct states | Queue at completion |
| --- | ---: | ---: | ---: |
| Two-entry retained prefix | 5,277 | 772 | 0 |
| Three-entry retained prefix | 32,005 | 3,416 | 0 |
| Stable inspection from every bounded history/writer state | 1,936 | 176 | 0 |

Early proof drafts retained their failures: one quantifier-scope error and
incomplete enabledness/temporal proof steps. The final proof supplies explicit
enabledness and fairness reasoning without weakening the invariant or adding
custom assumptions. Two initial source-control wrappers failed compilation
because dependencies and crate-private interfaces were inaccessible. The final
runner compiles an isolated complete Raft source tree against recorded Cargo
artifacts; compilation failure is never counted as a detected source fault.
The first formal runner also stopped at an incorrectly escaped mutation target;
that failed attempt remains separate from the corrected final run.
The second runner reached the real unfair-inspection counterexample but rejected
its one-state stuttering trace. The checker now permits that explicit shape only
when requested for a temporal counterexample; other callers keep their existing
two-state threshold. The original rejection remains retained.

No existing performance or Chaos cohorts were rerun. This API has no production
anchor consumer yet. Actual Chaos Mesh publication/recovery/installation faults
remain mandatory when that integration is implemented; previous runs do not
qualify this component as complete E2E anchor acceptance.

## Reproduce locally

Use the pinned TLA jar and TLAPS version from the inventory. Each output path
must be new. Keep build artifacts outside the evidence packet.

```sh
cargo test --offline -p kv9-raft --lib
cargo clippy --offline -p kv9-raft --all-targets -- -D warnings
cargo fmt --all -- --check
python3 -W error scripts/check-configuration-protocol.py \
  --jar /path/to/tla2tools-v1.7.4.jar \
  --tlapm /path/to/tlapm --output /new/configuration-proof
cargo build --offline -p kv9-raft --lib --features testing \
  --message-format=json > /path/to/cargo-artifacts.jsonl
python3 scripts/check-configuration-source-controls.py \
  --artifacts /path/to/cargo-artifacts.jsonl --output /new/configuration-controls
```

Next implement complete anchor validation using this history provider, exact
manifest publication evidence, root/range identities and durable retention
owners. Then compose the publication/quiescence/install proofs and exercise the
integrated paths under actual faults. The full write comparison against CRC
and the original multi-Raft/split dependencies remain unchanged.
