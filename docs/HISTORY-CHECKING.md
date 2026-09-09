# Independent Raw KV and catalog history checking

This is the C02 increment of [issue #12](https://github.com/c4pt0r/kv9/issues/12),
under [roadmap #9](https://github.com/c4pt0r/kv9/issues/9). It establishes an
independent executable model and concurrent fault histories. Complete protocol
refinement, production multi-chunk fault histories and the broader fault matrix
remain open. Transaction snapshot isolation is a separate T02 obligation.

## History and observable contract

`scripts/history/workload.py` invokes the public CLI. A single external
coordinator records invocation before starting each subprocess and response
after it returns, using a locked, increasing event sequence. Node clocks and
reported Raft positions do not establish ordering. Positions and raw CLI output
are retained for diagnosis. Every endpoint attempt has a distinct operation ID;
retries are visible calls. The client rotates among three stable replica
addresses after failures and remembers successful endpoints. It has no mandatory
seed or gateway. The Kubernetes client Pod has labels outside database fault
selectors; this harness coordinator is not part of the database service.

Version 1 JSONL starts with a header containing the seed, endpoint set, worker
count, initial catalog/KV state and production range chunk size of 1024. Each
invocation includes `id`, `client`, `op`, exact arguments, endpoint and phase.
Each response includes its matching ID, outcome, result, raw stdout/stderr,
exit code and receipt. A missing response means pending/unknown. Byte strings
use canonical lowercase hex; a missing value differs from an empty value.
Malformed successful CLI output fails the recorder and is rejected by the
checker even if a caller later submits the saved history manually.

The independent state is a finite mapping `(keyspace, key) -> value` and a
catalog mapping `name -> id`. No Rust database code, storage file, status file or
production model is imported into `checker.py`.

| Operation | Model transition and observation |
|---|---|
| `put` / `delete` | One atomic point mutation in an existing keyspace |
| `get` | One atomic observation of the current value or absence |
| `scan` | One atomic, sorted observation over `[start,end)`, bounded by `limit`; empty end is unbounded, zero limit returns no rows |
| `create_keyspace` | A unique nonempty name receives a previously unused nonzero 24-bit ID |
| `delete_range` | Select one immutable ordered key set after invocation, then delete its successive chunks atomically; completed chunk count must match the result |

Catalog checking proves name and ID uniqueness/non-reuse for the recorded
operations. It does not require gapless IDs or infer allocation order from
response order. Unknown creations may allocate any observed ID or a fresh
symbolic ID. Symbols represent distinct unobserved IDs and are bounded by the
remaining 24-bit domain; unobserved IDs are otherwise observationally symmetric.
The harness creates its own keyspace; unrelated fixture mutations use a separate
keyspace and do not affect its model. The initial fixture catalog is declared.

A confirmed atomic operation takes effect strictly after invocation and no later
than its response. A transport timeout, generic RPC error or generic NotLeader
is **unknown**, not proof of no write. An unknown mutation may be omitted or may
take effect at any later point, including after the observed timeout response.
Unknown reads have no confirmed observation and no effect, so omission is safe.
Only a separately proven pre-commit refusal has no effect; the recorder currently
emits no such refusal because diagnostic prose is insufficient evidence.

Range deletion follows `RuntimeBackend::raw_delete_range`, `run_delete_range`
and `RawExecutor::plan_delete_range_chunk`: one established snapshot supplies
all chunks. A later write to a selected key can be deleted by its later chunk;
a newly inserted key absent from that snapshot survives. Whole-request range
delete is not atomic, even when the selection fits one chunk. For example,
selecting `{a}`, then observing a completed insert of `b`, then reading old `a`,
then deleting `a` while `b` survives is a legal history. Modeling that request as
one atomic range mutation would incorrectly reject the implementation contract.
A typed partial result proves its completed prefix before the response and allows
at most one extra unresolved chunk. Generic unknown results provide no such
upper bound. Real-time linearizability claims apply to the atomic API subset;
range requests satisfy the explicitly weaker transition contract above.

## Search and certificate argument

A search node contains the observed-event cursor, the entire model state and
per-operation progress. Atomic progress is unstarted or done; range progress
contains the immutable selected keys and applied chunk count. The cursor advances
until a confirmed response (or typed partial prefix) needs more model steps.
Search then enumerates effects of already invoked operations, including unknown
mutations whose timeout response was already observed. It never partitions
overlapping point, scan and range operations into independent key histories.

**Soundness.** Initially the model equals the declared initial state. Every
certificate effect is an enumerated transition on that state; induction on the
certificate preserves the model transition relation and prevents duplicate
atomic effects. Replay rejects effects before invocation. Every response barrier
is checked before advancing past it and again at the end. Consequently each
confirmed observation holds at the indicated step; each required mutation or
partial prefix has completed before its response. Monotone event boundaries
imply that an atomic operation completed before another was invoked precedes
that operation's effect. Unknown writes have only an invocation lower bound.
`verify_witness` replays the complete certificate from scratch, independently of
DFS cursor retirement and memoization, before any valid verdict is returned.
Replay deliberately shares the model transition function; model correctness is
a separate obligation supported by the controls and source mapping.

**Unrestricted search completeness for this finite model.** Take any legal
execution of the finite recorded operations. Move its effects to the next
unsatisfied response barrier without changing their order. No intervening
confirmed response needs them (otherwise that response would have been the
barrier), and moving right does not violate invocation bounds. Effects after the
last observation are unnecessary and may be omitted. This yields a path in the
enumerated search tree. Each operation has finitely many effects and choices:
point effects occur once, each range selects once and advances through a finite
key set, and catalog choices use the finite observed-ID set plus symmetric fresh
representatives. States include exact KV/catalog contents, cursor and progress;
memoizing identical states preserves all possible suffixes. Retired completed
operations are excluded from future candidates by their observed response bound.
Thus exhaustive unrestricted search failure establishes no legal execution of
this model. A time, state or frontier limit yields **inconclusive** instead.

The checker first tries omitting unknown effects, then a guided attempt that
prefers unknown effects reducing the pending observation's mismatch (including
range-selection lookahead), small unknown-effect budgets (1, 2, 4), and finally
the unrestricted search. Restricted attempts are witness-finding heuristics only:
their negative results never establish invalidity, and every positive result
replays against the unrestricted model. The total time/state budget is shared.
A valid interpretation that omits an unknown write is not evidence that the
actual write failed. Search order may change without strengthening that claim.

Invalid histories retain a proven-invalid event prefix. Prefix minimization
preserves all preceding invocations and treats unfinished calls as unknown.
Arbitrarily removing a causal write could manufacture a stale-read example and
is therefore not used. Budget exhaustion keeps the last proven-invalid prefix;
the result need not be globally minimal.

`proofs/lean/History.lean` checks three unbounded lemmas: accepted generic
certificates form executions, executions are accepted, and operation interval
bounds preserve strict real-time order. The model includes explicit response
check actions. Mapping Python's implicit response checks to these actions,
Python execution to Lean's evaluator, and the concrete transition predicates to
the public API remain source-level refinement obligations. These lemmas do not
mechanically verify Python, the DFS completeness argument, raft-rs or the Rust
binary. Removing eligibility or weakening the ordering premise must fail Lean
for the expected reason. The complete inventory contains 12 lemmas with six
invalid proof controls.

## Executable acceptance

```sh
python3 -m unittest discover -s scripts/history -p 'test_*.py' -v
python3 scripts/history/check-controls.py --output /tmp/kv9-history-controls-new
cargo build --bin kv9
python3 scripts/history/local-e2e.py
python3 scripts/history/checker.py /path/history.jsonl --output /path/result.json \
  --acceptance --require put get delete scan delete_range create_keyspace
```

The checker exits 0 only for valid, 1 for proven invalid, and 2 for inconclusive
or malformed input. Acceptance requires successful mutations and reads and all
requested API kinds; zero operations, all errors and missing coverage fail.
Controls include 34 tests and 200 deterministic small atomic histories checked
against a separate permutation/subset oracle. Eleven isolated source mutations
each require exactly one selected test to pass, fail its intended assertion,
and pass after byte-for-byte restoration. Sources and mutant hashes, all phase
logs and exact test counts are retained. A compile/import error cannot count as
a caught semantic defect.

`scripts/chaos-mesh-e2e.sh` records a concurrent whole history across its actual
PodChaos, NetworkChaos and IOChaos matrix. Every sustained voter failure,
partition, delay and each voter/errno cell additionally requires an independent
successful put and read invoked after the fault effect was observed and completed
before healing starts. Phase progress is saved and the continuing fault is
checked after those operations. Final checker coverage requires all 13 named
windows. Pod/container kills are instantaneous events spanned by the history,
not intervals with an asserted in-fault operation. CI verifies explicit matrix,
history and per-phase completion markers and uploads all evidence on either
success or failure.

The randomized workload uses three concurrent workers, eight overlapping keys,
bounded scans/ranges, distinct written values, racing catalog names and a
reserved acknowledged key outside mutation ranges checked after healing. This
bounds search while preserving overlapping multi-key operations; it does **not**
exercise a production range larger than 1024 keys or force a typed partial range
result under faults. Synthetic controls cover those semantics; real multi-chunk
and partial-result fault histories remain C02 work. The current IOChaos cells
cover reopen/catch-up Raft writes, not all engine/checkpoint I/O. The one-host
Kind cluster cannot establish host-loss availability. See
[Chaos I/O limits](CHAOS-IO-VALIDATION.md) and the
[mandatory proof/fault/availability contract](CORRECTNESS-GATES.md).

## Local validation record (2026-09-08)

The complete revised Chaos Mesh matrix passed with 1,480 operations: 1,312
confirmed successes and 168 unknown results, all six APIs and all 11 required
fault windows. The checker replayed its witness in 0.389 seconds, visiting 1,444
states. Evidence is retained at `/tmp/kv9-chaos-e2e.C3gAGO`; the runner log is
`/tmp/kv9-c02-chaos-final.log`. The separate three-process acceptance recorded
240 operations (218 successful, 22 unknown), all six APIs, and passed witness
replay; artifacts are at `/tmp/kv9-history-local-vvcf3aki`.

Two earlier complete fault histories exceeded search limits and were correctly
reported inconclusive. Both original histories were retained unchanged. The
first (1,278 operations) exposed expensive branching over irrelevant unknown
read/write effects and passed after the bounded witness attempts were added.
The second (1,448 operations) needed an unknown put to explain a later confirmed
scan after leadership changed. Omitting every unknown effect was therefore
insufficient; the guided attempt found and replayed a full witness in 0.784
seconds. That run's original failure remains at `/tmp/kv9-chaos-e2e.n6jlzr`, with
the successful recheck at `/tmp/kv9-c02-phase-rechecked.json`. Neither history was
reduced to obtain a valid verdict. The 30-test suite includes a bounded-search
regression with many unknown writes and a late scan observation; restricted
negative verdicts remain explicitly non-authoritative.

The seven mutation controls passed baseline/mutant/restored checks, all 12 Lean
lemmas passed with six rejected invalid controls, and Bash syntax, actionlint and
whitespace checks passed. These local observations do not assert hosted CI
success; published issue evidence records the observed runs at their commit.

## Hosted integration follow-up

The first hosted CI run at `c72d550` failed the existing dynamic-membership E2E
at its one-shot post-failover CreateKeyspace call. The saved scene shows the
replacement candidate's local leader status in term 6 while its applied prefix
still belongs to term 5; the public RPC returned `not leader`. Local role status
was being treated as a guarantee that the next RPC would succeed. The failure
is retained in run [34288658386](https://github.com/c4pt0r/kv9/actions/runs/34288658386),
artifact `scene-dynamic-membership-e2e.sh-34288658386-1`.

`scripts/membership-write.sh` now bounds retries of that observed leadership
rejection, re-resolves the routing candidate and saves every attempt's endpoint,
start/end timestamps, stdout, stderr and exit status. It retries the same unique
name. A timeout, transport failure, duplicate-name response or missing successful
receipt cannot satisfy the write. The retry-start budget is 20 seconds and each
in-flight client call remains bounded by the existing 15-second timeout. The
existing exact term/index checks on every surviving and restarted voter remain
unchanged; no production protocol, fsync or quorum rule changed.

Four deterministic controls cover transient leadership rejection, timeout,
duplicate-name refusal and exhausted routing budget. A single-defect source
mutation disabling the leadership retry fails at the intended refusal; baseline
and restored controls pass. The full real five-voter join/promotion/failover and
restart E2E passed locally, with artifacts at `/tmp/kv9-membership.TekcYJ`.
Hosted validation of the follow-up is recorded separately in issue evidence.

Public admission refusals use the existing `refused`/`precommit` model outcome
only after the recorder checks an exclusive Raw CLI refusal, exit 1, empty stdout
and the optional exact kubectl trailer. Timeout or mixed output remains unknown.
The guided search assigns no observation distance to a refused read: it has no
value/rows result to explain. Both parser and heuristic have isolated source
controls; the full witness still uses the same sequential transition relation.
See [PUBLIC-ADMISSION.md](PUBLIC-ADMISSION.md) for the live overload fault window.

Unknown-write candidates are ordered from recent to old after the immediately
required operation and confirmed work. Older unresolved RPCs remain legal search
candidates. This avoids trying many obsolete range selections before a recent
unknown delete that explains the current observation. A fixed-budget known
witness test and its oldest-first mutation discriminate this ordering. No search
cap is increased, no transition is pruned from unrestricted search, and bounded
or guided exhaustion still cannot establish invalidity.

The guided pass prefers an eligible overlapping successful read when its recorded
result already matches the current state, before applying an overlapping write.
Otherwise a later read response can be explained by an unrelated old unknown
delete, causing avoidable backtracking when a subsequent read needs that write.
This changes candidate ordering only. A fixed-budget known witness and isolated
deferred-read mutation exercise this case; all positive witnesses are replayed
and unrestricted exhaustion retains its complete transition set.
