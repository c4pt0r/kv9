# Raft frame-buffer release and recovery qualification

The separate single-buffer Raft WAL candidate `01d128f` now passes its exact
default release and ordinary three-voter recovery gates. Independent checking
accepts **353 complete operations: 323 OK and 30 unknown**, across streaming
and unary RPCs. All six final replica drains pass; five server and two client
lifetimes exit. Actual Chaos Mesh and performance qualification remain pending.

The [source-qualified change](https://github.com/c4pt0r/kv9/blob/01d128fd771dfbf0e6826ee5b6821411afac1fec/docs/WRITE-RAFT-FRAME-BUFFER.md)
removes one allocation and body copy per Raft WAL record. It preserves length,
checksum, kind and payload bytes, the existing write call, error mapping, sync,
failed-writer and publication fences. Three universal SMT checks, three
counterexample controls, 710 workspace tests/doctests, formatting and Clippy
already pass; 23 existing tests remain ignored. The compatibility case writes
and replays 3,840 frames. Those prior populations are separate from this
release/recovery run; no new performance improvement is inferred from them.

## Exact release

| Artifact | Identity |
| --- | --- |
| Source | `01d128fd771dfbf0e6826ee5b6821411afac1fec` |
| Server SHA-256 | `53945784b39f7c90951b96d6f0700f432412369c24f26dcdd1d27e1988541c8c` |
| Native recovery client SHA-256 | `910d1e090d85a343a39052405c8ecee5037c3ca506a9d2e699c2b3555d5f57f9` |
| Release terminal | session 3608, `9285b7/0` |
| Independent release readback | `f47eb6/0` |

The retained build covers 657 exact source files from a clean checkout. Both
standalone binaries use the default feature graph, opt-level 3, ThinLTO and
one codegen unit, verified against verbose compiler commands and Cargo artifacts.
The original shared BuildCache lock and explicit first-party invalidation
remain in use, with four offline build jobs. Source and retained reference
releases remain unchanged. The original 96-GiB preflight, 80-GiB runtime floor,
16-GiB additional consumption ceiling, five-second samples and 1,200-second
outer deadline pass; minimum observed release free space is 121,465,839,616 bytes.

## Ordinary recovery

| Transport | Complete operations | OK | Unknown | Fresh final voter drains |
| --- | ---: | ---: | ---: | ---: |
| Default streaming | 175 | 161 | 14 | 3 |
| Explicit unary | 178 | 162 | 16 | 3 |
| Total | 353 | 323 | 30 | 6 |

The fixture exercises overlapping point and atomic batch operations, leader
loss and restart from the original storage directory. Both complete histories
pass native report checking and an independent raw atomic-history checker.
It retains successful traffic during voter-loss and restart windows, exact
writer/listener/process identities and fresh drained replica observations.
Unknown outcomes remain unknown; this does not establish that they all failed
or all committed. The process terminal is session 67872, `aa24b3/0`; the
unchanged independent audit passes `e57916/0` and confirms source/input stability
and all owned lifetimes exited.

Only source, build and output path literals change in the reused recovery
wrapper. Its seven source-relative helpers and generic auditor are unchanged.
The [original evidence and exact-byte archive](raft-frame-buffer-recovery-v1/README.md)
include the small process/WAL/history inputs, build logs, source maps, helper
diffs and actual terminal receipts. Retained executable payloads remain
separately hash-bound. No workload was rerun and no hosted CI was dispatched.

## Remaining gates

Run the candidate's actual 21-window Chaos Mesh matrix with the exact release
bytes, independent full histories, fault-effect checks and owned cleanup. Keep
the existing scope limits: a one-host fault campaign is not independent-host
or power-loss acceptance. Then measure throughput and tail latency against
the selected server with the fixed benchmark client and full regressions.
The checksum-slicing candidate remains separate until independently selected;
this frame-buffer result changes no default runtime or industrial checklist item.

The full CRC regression campaign also needs qualified storage capacity. The
[prepared matrix](write-crc-full-regression-plan-v1/README.md) retains its
24-smoke/48-timed scope and all original guards. Capacity work proceeds
independently of this release/recovery qualification.
