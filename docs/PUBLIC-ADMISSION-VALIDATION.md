# Public admission validation record

Issue [#38](https://github.com/c4pt0r/kv9/issues/38). The protocol, accounting units
and remaining obligations are in [PUBLIC-ADMISSION.md](PUBLIC-ADMISSION.md).
Exact pushed-revision CI and artifact acceptance are recorded on that issue and
the [roadmap tracker](https://github.com/c4pt0r/kv9/issues/9).

## Local pre-push evidence

- The workspace passed 444 ordinary tests and 20 doctests. The 21 object-store
  integration tests remain separate live-MinIO gates, not silently counted as
  locally executed ordinary tests. Clippy, formatting and actionlint passed.
- Eleven new Rust admission tests cover both limits, overflow/saturation,
  configuration, all 25 handlers, validation, completion, error, panic, queued
  cancellation and authenticated real-wire count/byte refusal with shared-listener
  Raft discovery progress.
- Three isolated Rust controls each ran baseline, intended assertion failure and
  restored source. The controls remove the count bound, aggregate byte bound, or
  reservation ownership in the actual blocking job.
- Seven new Lean resource declarations passed. The full fresh inventory passed
  24 declarations and nine invalid controls with its transitive axiom audit.
- The history suite passed 34 tests and eleven isolated source mutations. New
  cases cover exclusive pre-execution refusal, refused reads without value
  observations, recent unknown-write ordering and overlapping read ordering.

The complete local Chaos Mesh run at `/tmp/kv9-chaos-e2e.aBacdI` passed all 13
fault windows, six voter/errno I/O cells and independent full-history replay.
Its history contained 1,918 operations: 1,669 confirmed, 27 explicitly refused
and 222 unknown. None of the unknowns were erased. The pressure fixture recorded
7,809 reads: 961 successful missing-key reads, 6,847 count refusals and one
oversized-request refusal. The observed peak was eight jobs and 352 encoded
bytes under the configured eight-job/2-MiB limits; reservations drained to zero.
Four invalid pressure-evidence controls failed their intended assertions.
This run is three voters plus a learner on one Kind host, not a host-loss test.

## Failed attempts retained and investigated

The first run reached real pressure and history progress but failed its evidence
parser because GNU date used a comma before fractional seconds. The parser now
accepts both ISO decimal separators. The run is retained at
`/tmp/kv9-chaos-e2e.mCcCj4` and is not counted as a complete matrix pass.

The second run completed fault execution but its history search was inconclusive
under the existing state/frontier budget. That original artifact remains at
`/tmp/kv9-chaos-e2e.7nvwSk`, including its original verdict. Investigation found:

1. The workload recorder had not yet classified the new exclusive admission
   marker as the model's existing proven-refusal outcome. Timeout and mixed
   outputs must remain unknown.
2. Guided search tried to read a value/rows field from a refused read. Such a
   refusal has no observation to explain.
3. Preferring an overlapping write before an already matching read could force
   the search to try many unrelated old unknown deletes to explain that read's
   later response. Candidate ordering now prefers eligible matching reads in the
   guided pass and recent unknown writes before older candidates.

A proposed additional range-selection guard was discarded: the transition model
already enforced that constraint, and the intended mutation did not fail. It is
not included in the implementation or the proof/control counts.

The final checker independently replayed a legal full witness for the **original,
unchanged** second history: all 1,825 operations, including all 239 original
unknowns. It visited 3,301 states in approximately 1.6 seconds using the unchanged
60-second/200,000-state acceptance budget. No operation was omitted from the
history, no outcome was reclassified in that replay, and no search budget was
increased. The unrestricted transition set and witness evaluator are unchanged;
restricted/guided exhaustion still cannot prove invalidity. The later complete
matrix pass and exact-revision hosted run are separate evidence.
