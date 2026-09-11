# CRC native link/quorum acceptance

All three frozen read-only checks passed once after runtime session 41221 exited 0. Check session 11116 exited 0; every owned check process is terminal. Exact source `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`.

Complete atomic history: **2,068 calls**, **1,761 OK / 232 refused / 75 unknown**. The 4,137 JSONL records contain one header and 4,136 operation events. The original unsuccessful zero-unknown-effect search and subsequent valid guided witness remain retained.

| Window | OK | Refused | Unknown |
|---|---:|---:|---:|
| baseline | 132 | 0 | 0 |
| vip-delay | 132 | 0 | 0 |
| delay-healed | 136 | 0 | 0 |
| vip-partial-loss | 194 | 0 | 4 |
| loss-healed | 140 | 0 | 0 |
| vip-partition | 0 | 0 | 36 |
| partition-healed | 144 | 0 | 0 |
| exact-tcp-reset | 148 | 0 | 0 |
| reset-healed | 152 | 0 | 0 |
| quorum-loss | 0 | 136 | 0 |
| quorum-healed | 156 | 0 | 0 |

Window counts are fully contained calls, so they do not sum to the complete history.

The same native Pod/container/netns and netem leaf `5:` (parent `1:4`) advanced **0 → 158 = 158 drops**. Parent counters are not added. Both original qdisc paths and SHA-256 values are in `leaf.json`.

Six post-exit observations cover all three voters and pass the unchanged two-fresh-empty-export drain gate. Exact source/binary, default features, image, Pod/PID/start/boot and batch activity checks passed. Native writer lifetime stayed constant.

Retained cleanup commands confirm all five owned container lifetimes exited, the exact namespace UID `3bc58cea-caa4-4930-9823-070abf5761cc` is absent, all eight historical namespace UIDs match, and both owned node directories are absent. The unchanged audit rehashed 50 retained files (12,382,528 bytes).

Retained timed server plus separately built same-source native correctness workload; default release Cargo graphs, not a combined server build.

Exact-source one-host volatile-tmpfs native client-link/quorum correctness; 11 original windows. TCP socket destruction is a non-Chaos exact-tuple reset. No throughput, disk/power-loss durability, full 21-window replacement, inter-voter partial loss or storage-stall acceptance claim.

Frozen result hashes:

- `audit.json`: `e7bc08b7006a01b287f147d7b1265382fdcc505dfdaf1b5ae54117126ce1ea79`
- `leaf.json`: `42e14438796caa1c3ee8776a9d5e12004aa5e779725795383a640950dc8aa6ea`
- `source-final.json`: `a341ba445a5273ff1a836e8a26a143be35b11547cde32f8d00c4965e07de5096`
- `preflight-first.json`: `426ce3b9ccac8c639ed8de56d5ee1ab8f7dba112bcb2c623e283b39210e032d7`
- `checks-terminal-first.json`: `19d0c1754492e588eba2751e835039b2d6fae318e4943a1213b7e6e2ea40043d`
