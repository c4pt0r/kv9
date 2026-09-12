# Coalesced owner notification validation

Candidate [42e0117](https://github.com/c4pt0r/kv9/commit/42e0117b13bed9671b672436c6b36bfe63c462af)
passes the local scheduling proof, source and ordinary recovery gates. Selected
runtime behavior remains CRC `ca0002c7`; this report does not promote the candidate.
The [implementation and proof mapping](https://github.com/c4pt0r/kv9/blob/42e0117b13bed9671b672436c6b36bfe63c462af/docs/COALESCED-OWNER-NOTIFICATIONS.md)
and [portable original evidence](coalesced-owner-validation-v1/README.md) define
the precise scope.

The only runtime difference from CRC is the `!pending` condition inside the
existing `WorkSignal::notify` mutex. It suppresses duplicate physical wakes.
Queue publication, consume-before-drain, predicate-protected parking and terminal
stop remain unchanged. No new consensus or persistence authority is introduced.

## Completed local gates

| Gate | Result |
| --- | --- |
| New TLAPS refinement | 14 theorems, 64 obligations |
| Fresh unchanged scheduling dependency | 33 theorems, 294 obligations |
| Finite scheduling checks | 1,300 / 1,723 states, both fingerprints |
| Conditional fair continuation | 241 states |
| Missing-first-wake controls | Model and proof reject the mutation |
| Proof audit controls | Omitted proof and injected assumption rejected |
| Proof output controls | Empty, zero-obligation and missing-summary outputs rejected |
| Default Raft/Server tests and doctests | 438 passed, one existing ignored |
| Raft testing-feature tests and doctests | 214 passed, overlapping the default population |
| Formatting and all-target Clippy | Both configurations pass, warnings denied |
| Recovery harness contracts | Five passed |
| Streaming/unary ordinary recovery | Independent complete-history audit passes |

The new Rust tests coordinate concurrent producers behind a pending hint,
publication during a bounded drain, and a parked owner stopped before later
publishers attempt to restart it. They test delivered work and terminal behavior;
they do not count physical wake system calls. Two incomplete development proof
attempts remain in the archive alongside the corrected, fully accepted proof.

## Original release and recovery

The original default release at `/tmp/kv9-coalesced-owner-release-first` is
cleanly bound to `42e0117`. Rust 1.94.0, commit
`4a4ef493e3a1488c6e321570238084b38948f6db`, matches the original CRC compiler.
The shared build transaction records 20 first-party observations, including
11 first-observed compiled units, after explicit invalidation. The source gate
and committed release differ only in the validation documentation; runtime,
test, model and proof bytes match.

| Original artifact | SHA-256 |
| --- | --- |
| Default server | `a9d7394466a6edf59e3fa8ac08b2ac4f4c51504bd0c5c02b207b1ad841b4717b` |
| Server build manifest | `1076ed28118a8d67650e489a093576793de7ada8aabdfaa6814db9026c68c1de` |
| Same-source recovery workload | `476f13a09f9ec75a18039decc2322a62b292ac304e43c9bdc5092725b8fc0f44` |
| Workload build manifest | `16678fd4a044db4bd01ea0c7840ff69c9eef94b2bd16389316ec2b75e3329e82` |

Both transports retain overlapping point and atomic batch histories through
leader loss and restart using the original data directories. The streaming
case checks 186 operations (170 OK, 16 unknown); unary checks 183 (171 OK,
12 unknown). All 369 operations are included; the 28 unknown outcomes are not
silently turned into failures or successes. Both native and raw history
checkers accept the complete histories. Six fresh voter drains are checked.

Source qualification is terminal at root session 15738 / `17ab70`; the original
release at 52415 / `0e4625`; ordinary recovery at 81153 / `bd4bdc`; independent
audit at `1f4292`. These local receipts and their original logs are archived.
Archive verification checks 278 original files / 5,069,166 decoded bytes without
rerunning the gates.

## Remaining acceptance

The abstract proof depends on the documented single-owner mutex/condition-variable
semantics and conditional service fairness. It is not a mechanized Rust/compiler
refinement, an operating-system scheduling bound or a full database proof.
Ordinary local process recovery does not establish actual candidate Chaos Mesh,
independent host loss or complete industrial failure coverage. Those requirements
remain open, and actual candidate Chaos Mesh is required before promotion.
Performance must be judged from its separately audited matched experiment.

All work ran locally. Hosted CI remains manual and no workflow was dispatched.
