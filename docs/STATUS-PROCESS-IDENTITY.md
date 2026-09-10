# Process identity in persisted status

Tracking: #9 and #13. A restarted process must not inherit the previous
process's status counters merely because it reuses the same numeric PID.

## Observed failure

The exact async-write Chaos attempt at `/tmp/kv9-chaos-e2e.5qdh24` completed its
original 21 fault windows and independent history/effect checks, but its extra
status observer rejected a purported peak decrease from 1 to 0. Both probes
reported PID 1 with the new process's executable and start ticks. The first
status record was actually left on the PVC by the old process: its export count
was 37; the subsequent record from the new process had export count 1. Linux
reported different process start ticks for the old and new executions.

Checking the current executable and PID before/after reading a file does not
establish which process wrote that persisted file. A Pod UID can also survive
a container restart. The failed observation and its original inputs remain
retained; they are not reclassified as a valid within-process counter reset.

## Additive status contract

Every newly written runtime status includes:

- `process_start_ticks`: Linux `/proc/self/stat` field 22, in kernel clock
  ticks since boot, captured for the process that creates the runtime exporter.
- `process_boot_id`: the Linux boot ID, distinguishing the clock domain across
  reboots.

The existing `pid` and node/store identities remain. Capture occurs once per
exporter construction. Neither public requests nor repeated status publication
read procfs. Input reads have a 4 KiB bound; malformed, missing or unsupported
identity evidence renders the literal `unavailable`. Diagnostics do not make
startup depend on procfs availability. The metrics JSON schema is unchanged.

An external observer must bind both status snapshots to the actual process's
PID, start ticks and boot ID, within the same node/Pod/container context, before
using their counters. Missing fields, `unavailable`, mismatched identities,
container transitions or an unobservable process are insufficient evidence.
Keep actual per-process monotonicity and occupancy checks unchanged after this
binding. A stale record may remain on disk until the next publication; these
fields let a consumer identify it rather than assume a reused PID proves it
current. Older binaries cannot supply this additional evidence retroactively.

The identity is a local diagnostic discriminator under the operating system's
process/boot model. It is not authentication, consensus authority, a store
incarnation, a new failure detector, or proof of independent host availability.
Status is still best effort and not one atomic snapshot of all runtime state.
The production CLI creates one node runtime per process; applications that
replace a runtime inside a still-live process need an additional counter-reset
domain before interpreting those separate runtimes as one monotonic stream.

## Validation scope

The focused tests cover reused PID 1 with the two observed start positions,
process names containing spaces/newlines/parentheses, mismatched PID, truncated
or overflowing start evidence, invalid boot IDs and explicit unavailable
rendering. A Linux test compares the exported identity with the live process.
Existing metric-export failure/progress and document-size tests remain intact.
The real-process fixture must also bind the new status fields to each executing
process across failover and original-directory restart. The updated fault
observer must reject stale same-PID records before a new exact-source Chaos run
can provide its stronger acceptance claim.

This patch changes diagnostic identity only. Raft/apply/read/admission,
durability, receipts and retry semantics are unchanged. The independent
async-write attempt's missing delay envelope remains a separate observation
coverage failure; this product patch does not make that earlier run accepted.

### Local main-line validation, 2026-09-10

On the accepted main runtime based on `9ada351`, all 591 workspace
tests/doctests passed, with zero failures and 23 ignored. All-target Clippy with
warnings denied, formatting and diff checks passed. The focused observability
suite passed all four tests, including both new identity cases.

The unchanged three-process RawKV fixture passed failover, subsequent writes,
delete and original-directory restart. All four observed executing lifetimes
had both status snapshots bound to their actual PID/start/boot identity and
default executable SHA-256
`5d5aa8772bd6946f52164634b12d554c1420cc7f017906b9591e2870867891df`.
The root, engine, Raft and server package feature lists were empty. Every owned
process exited; source and executable bytes remained unchanged during the run.
An observer replay using the actual retained old/new status records rejected
an old record with only its PID replaced to simulate reuse; missing and
unavailable identity fields were also rejected. This replay is not an actual
PID-namespace restart. Raw records and the observer are retained at
`/tmp/kv9-status-process-identity-process-first` and
`/tmp/kv9-status-process-identity-process-run.py`.

The later async-write runtime needs its own integration validation and exact
fault observer acceptance. No earlier binary is relabeled with this evidence;
no hosted CI was dispatched.
