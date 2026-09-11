# Async native batch-read Chaos Mesh acceptance

The exact asynchronous native batch read candidate passed this scoped local Chaos Mesh run and its existing independent read-only audit. The full 21-window matrix and formal refinement gates are separate. No performance or broad issue-completion claim is made.

Runtime/source: `af4c4e31bdef2b1294931c27e9802bc04e6aeaf5`, clean `/tmp/kv9-async-native-batch-read`. Original standalone release build: `/tmp/kv9-async-native-batch-read-release-first`. All 578 source files and both default-feature dependency graphs were rechecked unchanged after execution. Server SHA-256 `8217d3520ee719c548754296518fabd8e20d82aa5fb78a8ada2a1e526c98a758`; native client `0b5116b72e6962a6d881269219d4451300aab7253f859de4ee11caddcd1a11f0`.

Minimal pinned-Ubuntu image tag `kv9-chaos:native-batch-af4c4e3-release-20260910-attempt1`; Docker/CRI config ID `sha256:96b01a450c280cfeadda0084f32929c970cf8129c1912d7cd2986a0938b929c3`; actual CRI manifest `docker.io/library/import-2026-09-10@sha256:36ff7b26c675acc918c265acfc0214290e904391ea1dbf6c15ab9ea2d46eef1b`. The complete payload/config/import binding is retained in the preparation directory.

## Outcomes

The unchanged complete-history validator accepted **2,059 calls / 4,118 events: 1,755 OK, 80 unknown, 224 refused**. Unknown writes number **44**, including **29 BatchPut** calls. The first search permitting zero unknown effects was invalid; the subsequent permitted atomic unknown-effect search produced a checked witness. Total search states: 5,410. Unknowns were retained without retrying uncertain writes or treating them as definite failures.

| Kind | OK | Unknown | Refused |
| --- | ---: | ---: | ---: |
| batch_get | 616 | 28 | 78 |
| batch_put | 613 | 29 | 78 |
| delete | 176 | 7 | 22 |
| get | 176 | 8 | 23 |
| put | 174 | 8 | 23 |

The following counts include only calls whose complete invocation/return intervals are contained inside each anchored window. Phase labels also cover setup/transitions and are not used to infer the negative interval. Unknown and refused populations remain in the full history.

| Window | Seconds | BatchGet OK / unknown / refused | BatchPut OK / unknown / refused |
| --- | ---: | ---: | ---: |
| baseline | 16.376 | 47 / 0 / 0 | 46 / 0 / 0 |
| vip-delay | 24.635 | 47 / 0 / 0 | 45 / 0 / 0 |
| delay-healed | 16.814 | 46 / 0 / 0 | 46 / 0 / 0 |
| vip-partial-loss | 35.194 | 60 / 5 / 0 | 60 / 5 / 0 |
| loss-healed | 17.513 | 49 / 0 / 0 | 49 / 0 / 0 |
| vip-partition | 17.877 | 0 / 13 / 0 | 0 / 13 / 0 |
| partition-healed | 18.025 | 50 / 0 / 0 | 51 / 0 / 0 |
| exact-tcp-reset | 18.380 | 51 / 0 / 0 | 49 / 0 / 0 |
| reset-healed | 18.770 | 53 / 0 / 0 | 53 / 0 / 0 |
| quorum-loss | 18.982 | 0 / 0 / 48 | 0 / 0 / 48 |
| quorum-healed | 19.335 | 55 / 0 / 0 | 55 / 0 / 0 |

**Neither negative window contains any newly successful operation of any kind.** Client partition contains 13 unknown BatchGet and 13 unknown BatchPut calls; quorum loss contains 48 refused BatchGet and 48 refused BatchPut calls. The corresponding unaffected physical/control paths were checked, and healed windows resumed successful native batch reads and writes.

## Injected effects and execution identity

- Service UID/VIP/port and ready EndpointSlice/Pod UID bindings were retained and rechecked. VIP delay was actual 250 ms netem: selected TCP probes ranged 251,178–251,644 microseconds, while simultaneous unaffected probes stayed at or below 1,844 microseconds. These are effect observations, not performance measurements.
- Genuine partial loss was 30%, correlation zero. The selected netem counter increased by 332 drops and TCP retransmissions by 112, while control-client/voter paths passed their unchanged checks.
- Full native client partition advanced reachable DROP counters by 66, with exact kernel target sets and failed selected VIP probes. Quorum loss advanced per-voter DROP counters by 46 / 42 / 46; only voter-to-voter edges were selected.
- The TCP reset is **non-Chaos Linux SOCK_DESTROY**, constrained to one owned IPv4 tuple. The native client remained PID 32, start tick 137278234, boot ID `455869d6-cdc2-4933-85cb-743fb8fcb02e`. Its socket changed from inode 174530649, source port 50502, to inode 174565423, source port 58938, against the same VIP `10.96.86.249:20160`.
- All observed native/server CPU masks were `6-15,22-31`. Source defaults of 64 public requests and 64 MiB encoded admission bytes were independently checked against actual Pod specs and statuses; no Pod limit overrides were set.
- Thirty-eight server observation envelopes and 25 native client captures retained exact executable/PID/start/boot and Pod/image bindings. The batch diagnostic delta was **616 inline completions and 1 blocking submission**, all on voter 1. This is the whole fixture envelope, including setup/verification; it is not per-response or per-window attribution. Blocking fallback is permitted.

## Drain, retention and cleanup

The native client exited 0. Five post-exit status captures established the required two serial fresh empty Serving publications per replica; both snapshots were empty, nonfatal, and read/apply registries remained running. Replica writers then stopped before stable data retention and archive readback.

Run session 9441 exited 0; runner PID 2871330 is absent. Namespace `kv9-native-link-acceptance-af4c4e3-20260910-1`, UID `06ea4d81-da7a-4258-bd1b-aa9c3ca4176b`, is absent. All eight historical namespace UIDs remain unchanged. All five captured node/container lifetimes are absent and containers removed/exited. Owned data and observer directories are absent. The independent audit session 49228 also exited 0, first attempt.

## Evidence and preserved attempts

- Raw run: `/tmp/kv9-native-link-acceptance-af4c4e3-attempt1`; log `/tmp/kv9-native-link-acceptance-af4c4e3-attempt1.log`.
- Prepared source, corrected image probes and exact release: `/tmp/kv9-async-native-batch-read-chaos-preparation`.
- Independent audit and arithmetic/cleanup supplement: `/tmp/kv9-native-link-acceptance-af4c4e3-independent`.
- The fault fixture passed its first run. Earlier preparation adapters remain failed and preserved: the first rejected Docker's omitted empty Cmd key; the second assumed the workload had a help option. The third image-only attestation passed against unchanged image bytes after checking the actual application-level missing-config behavior. Both owned probe containers were removed. A read-only query typo and correction are also retained. The older fixture's historical setup failures remain linked by source lineage; they are not new candidate failures or acceptance.

| Artifact | SHA-256 |
| --- | --- |
| Plan | `a2468f4fe9944808890af78e4634ac892ac8ec1489861f8951fa859a326e3367` |
| Independent audit | `8e5bfe11b04d72e255242d1883b54a6060a2d1d60e286326e9d241f9aae133f0` |
| Supplement | `a298265531b21d162e69290d49d6deb54c030d909b6df9ba4519169d2f31b7e3` |
| Native full history | `5c1fa52667713cafd0b583672482f381288ba8f33f02c69f1f497727da2f959c` |
| Native report | `de469465e578b5cb24888e7dfb9f35be8885d77ed9a16d6c7bae7341b833dc19` |
| Raw summary | `218cd8c83162dd3cedc31b18e3f8a093dee6bd99b379eda3f6979bd226531c56` |
| Cleanup | `6677c721a4b1bfc4d9bcc5607bd309fdbf68faac122f6bbd540925b1aca2fc30` |
| Owned data tar | `24b52f315f4b1c3838079498b320c703739cedced8ba5d31435777980099518a` |

The node data filesystem is volatile tmpfs on one shared Kind host. This evidence does not establish disk/power-loss durability, cross-host availability, throughput, liveness, the full fault matrix, or whole-adapter machine refinement. No hosted CI ran and no repository/GitHub mutation was made by this testing task.
