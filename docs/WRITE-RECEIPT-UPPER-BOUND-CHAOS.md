# Upper-bound receipt lookup: actual Chaos Mesh acceptance

Candidate
[`e2e23cc`](https://github.com/c4pt0r/kv9/commit/e2e23cca5e70a9ea0cc241877b3b35b5b6433d27)
passes the complete 21-window actual Chaos Mesh matrix, independent complete
history checks, full local archive/readback and exact-owned cleanup.
**CRC remains selected. The original matched performance screen and live
schema-2 observer capture remain pending; this checkpoint establishes no QPS
improvement or Redis parity.**

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,550 | 4,964 | 565 | 21 |
| Persistent point stream | 1,475 | 1,448 | 18 | 9 |
| Native point/atomic batch | 2,614 | 2,570 | 31 | 13 |
| Total | 9,639 | 8,982 | 614 | 43 |

Every invocation has a recorded return. All unknown outcomes remain in the
complete consistency checks. The workload includes successful reads and writes
inside the fault windows, with independent observations of the fault effects.
It covers registration-seed blackholing, each voter failing, leader partition,
public admission overload, network delay, EIO/ENOSPC on every voter, missing-log
and replacement-PVC refusal on every voter, and pending/recovered endpoint
migration. Formation crashes and container restart also pass.

Four final replica lifetimes each publish two fresh drained statuses.
All 36 observed server lifetimes and 25 containers exit. The owned namespace
is absent; all eight historical namespace UIDs remain unchanged. Quorum,
WAL synchronization, application, publication and client-response fences
retain their original requirements.

## Exact candidate and actual execution

All 1,116 clean candidate source files bind to the retained default server
SHA `6f074e867eae45274c17b54aa763888e92db2c45d5757974fa52ee0f15936d49`.
The image ID is
`sha256:451b98047cc2628fc062af6091bed425e372b04f01d094c0ece2ddf125b0807c`.
The default server, point client and native client all use empty optional-feature
sets. The separate remote pressure example has the accepted testing feature
graph and does not instantiate the database. The diagnostic server is separate
from this default-feature fault campaign.

The [source proof and development checks](WRITE-RECEIPT-UPPER-BOUND.md) and
[default/diagnostic releases plus ordinary recovery](WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md)
precede this gate. Actual same-source auxiliary builds (`73932/569c0f/0`),
independent codegen readback (`cc5fe8/0`), image/probe/Kind loading
(`71001/8a874b/0`), finalization (`67e8f1/0`) and prebuilt verification
(`c7861c/0`) bind the one-use runtime release. The image's payload and loader
probe runs without network access, and its temporary container exits.

Runtime `94911/dafc9c/0` and post checks `90186/11ea7e/0` are terminal.
All six post phases pass: independent audit, cleanup capture, process-tree
readback, full archive/readback, exact-UID cleanup and all-lifetime readback.
No failed runtime was rerun. The original audit precedes cleanup and retains
`cleanup_complete=false`; the subsequent cleanup records complete that scope.

The inherited audit's free-text scope still names predecessor `a6ac335`.
Its machine revision, source map, image, binary and process fields bind the
actual upper-bound candidate above. This inherited label is preserved in the
original record and is not used as candidate identity evidence.

## Capacity and preserved preparation failures

The unchanged Chaos policy requires 20 GiB + 8 MiB available at launch, an
8 GiB floor and at most 12 GiB sampled decrease. Runtime and all post phases
share the same 26,883,084,288-byte finalizer baseline. All 120 samples satisfy
the resulting 13,998,182,400-byte floor. Minimum observed availability is
25,882,824,704 bytes; maximum sampled decrease is 1,000,259,584 bytes.
These are observations and guards, not a reservation for later performance
work.

Before finalization, two root metadata preparations stopped without launching
the fixture. The cold controller uses `state: COMPLETE`, while the finalizer
expects `complete: true`; an explicit adapter binds the original successful
terminal, original status and complete five-cohort acceptance summary.
A root process scan then classified the pre-existing system Redis daemon as a
benchmark. Its actual PID/start/cgroup identify the unchanged host service;
the corrected scan preserves that service and checks for active test work.
Both failures remain retained. No storage policy, fault predicate or historical
success receipt was weakened or fabricated to continue.

## Evidence and next work

The complete local archive has **4,552 members**, including 4,250 original
files / 923,659,003 file bytes, in **92,332,752 compressed bytes**. Every member
was read back and every original file rehashed before cleanup. Archive SHA:
`9c069050bbaf3e88f8859151dac1ba5f88fc618cdaea49aa1a73ac21dc9d5ab1`.

Portable selected histories and acceptance records are documented in
[the evidence packet](receipt-upper-bound-chaos-v1/README.md). Large WAL/ELF
payloads, duplicate audit copies and observer streams retain their complete
local archive authority; the portable selection does not replay every original
payload check.

The original eight-smoke/sixteen-ten-second matched screen is prepared with the
selected CRC reference and fixed native v3 client. Actual source, binary,
compiler and default-feature role readback passes (`d48f6f/0`). The established
initial capacity scenario is 79,455,850,496 bytes; it remains subject to fresh
space and every original per-phase guard. No smoke or timed cohort has run for
this candidate. Capture actual upper-bound skips separately before drawing
path-level conclusions; synthetic observer controls are not runtime evidence.

A subsequent live filesystem check finds 25,415,081,984 bytes available
(23.67 GiB), leaving about 50.3 GiB below that approximately 74 GiB empirical
scenario. The existing finite cache and lossless-retention candidates cannot
close this gap even at their zero-cost allocation ceiling. Additional storage
for retained evidence is needed before the full screen; 100 GiB available is a
planning target with iteration headroom, not a measured worst-case bound.
Availability must be refreshed at launch, and all original phase guards remain.

The separate [TCP fixture port repair](tcp-fixture-port-ownership-v1/README.md)
passes its exact test and formatting on main. It changes no production runtime
or frozen candidate artifact and does not retroactively pass the earlier
failed diagnostic suite.

This acceptance does not establish cross-host or physical power-loss tolerance,
dedicated client-link/quorum-loss coverage, or a whole-Rust/Raft proof. No
original industrial roadmap checkbox closes. Continue the measured write
decision, industrial storage prerequisites and dynamic multi-Raft/split route
in issue #9. All work remains local; no hosted CI was dispatched.
