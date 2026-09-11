# Repaired f2 follower storage-stall qualification

The separately released repaired controller session **25663 exited 0**, with all three runtime windows, original full-history validation, FIFO-aware archive retention and cleanup accepted. The independent retained-evidence audit also passed. This is exact f2 correctness evidence; the earlier controller's FIFO-extraction failure remains unchanged and separate.

The runtime/image, workload, CPU mask, fault delay and downstream consistency predicates were unchanged. The only executable repair was an explicitly hashed private retention helper that records the known supervisor FIFO as metadata while preserving the original tar and applying the existing data filter to every extracted member.

All **655 logical calls succeeded**, with no unknown/refused terminal outcomes: 66 GET, 65 PUT, 64 DELETE, 231 BatchGet (**1,842 items**) and 229 BatchPut (**1,829 items**). Counts include initialization and verification; batch item counts preserve repeated keys. Ordinary SDK routing attempts remain subject to the original report checks.

| Window | Duration | Contained successful BatchGet | Contained successful BatchPut |
| --- | ---: | ---: | ---: |
| Baseline | 12.181 s | 33 | 34 |
| Raft WAL sync stall | 20.193 s | 55 | 57 |
| Healed | 12.203 s | 33 | 34 |

Actual `IOChaos` applied 100% FSYNC latency of 500 ms to voter-3 container `fixture`, path `/data/raft/raft.log`. Fault UID **`0ef4bf5f-6f33-43eb-8ee7-14895cd62524`** remained stable. The reopened writer was PID **408**, start tick **138475357**, FD **4**, FUSE mount **8077**, device **0:752**, inode **659958**. Raw Pod/CRI, process/namespace, executable, mount and descriptor records agree; that same descriptor appended new WAL bytes.

The fresh metric interval bracketing the stall window contains **42 additional successful record-sync samples**, total **21,053,222,637 ns**, mean **501.267 ms**. There were no new sync errors, cancellations or fast-bucket samples. All samples occupy the coarse **268.435456–536.870911 ms** bucket; the independent sum supports the mean without asserting exact per-call 500-ms durations. SIGSTOP was not the fault effect.

The writer stopped **34.343 s** after the conservative pre-create clock; the fault was absent by **34.534 s**, before the 70-second watchdog cutoff and 90-second TTL. No watchdog/forced-stop failure occurred. Independent majority service produced receipt index **138**; all healed voters caught up through it. Six post-client drain samples passed the original fresh-empty-export checks.

The audit replayed **1,398 raw commands**, six native captures, eighteen server samples, eight I/O captures and both explicit writer transitions. All **75 archive members** were checked: **58 regular files / 4,564,302 bytes** match the tar and retained inventory; the one FIFO's metadata matches without recreating a special file. Original cleanup verified all five Pod/CRI lifetimes exited, removed both exact-owned directories and namespace UID **`23bf0f5d-fe81-4cae-ad10-ede4148afb29`**, and preserved all eight historical namespace UIDs.

The first independent audit is retained as failed because its final report expression assumed a local image-binding file; the ready plan intentionally references the original accepted binding. The second audit changes only that reporting expression to use the plan's explicit path. All history/effect/retention/cleanup predicates are unchanged.

Scope: default-feature source **`f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02`**, one follower, WAL backed by node tmpfs. This is actual syscall-delay consistency qualification, not persistent-media/power-loss durability, leader/all-voter stalls, performance acceptance, or acceptance of later typed/allocator sources.

- Runtime: `/tmp/kv9-native-link-acceptance-f2-storage-stall-repair-attempt1`.
- Raw archive SHA-256: `187b87297d788d4bc76e486fb340883bffd69cd1775b15b67c6bbcbc246707ee`.
- Accepted independent audit: `audit.json`, SHA-256 `b5e0770bde93094d2cf8824aed6ee2544acf6d80a68e740838a7ce09e0b114e7`.
- First reader failure retained at `/tmp/kv9-storage-stall-f2-repaired-independent-first`.

All execution was local; no hosted CI was dispatched. See the [original failed-run record](STORAGE-STALL-FIRST-QUALIFICATION.md) for the retained first attempt. This increment does not complete the broader persistence and fault work package.
