# Owned-buffer process and client-link/quorum acceptance

Exact clean source `9be0c1963515ff974426faadef80482b847b13a5`, 592 source files. Original default release helper built the standalone correctness workload then server; no retrospective assembly or previous-server relabeling.

Process E2E session 55048 and its unchanged independent auditor exited 0: **367 calls, 331 OK / 36 unknown / 0 refused**. Stream:190 (172 OK/18 unknown); unary:177 (159 OK/18 unknown). Both full histories, leader-loss/original-directory restart windows and fresh three-voter drains passed. All five server and two client lifetimes exited.

Chaos session 1870 exited 0 (`17675a`); all three independent checks in session95613 exited0 (`2d6db8`). **2,047 calls:1,740 OK /220 refused /87 unknown**. The inconclusive zero-unknown-effect search and valid guided witness are both retained.

| Window | OK | Refused | Unknown |
|---|---:|---:|---:|
| baseline | 132 | 0 | 0 |
| vip-delay | 132 | 0 | 0 |
| delay-healed | 136 | 0 | 0 |
| vip-partial-loss | 158 | 0 | 21 |
| loss-healed | 136 | 0 | 0 |
| vip-partition | 0 | 0 | 38 |
| partition-healed | 144 | 0 | 0 |
| exact-tcp-reset | 148 | 0 | 0 |
| reset-healed | 148 | 0 | 0 |
| quorum-loss | 0 | 136 | 0 |
| quorum-healed | 156 | 0 | 0 |

Window counts are fully contained calls and do not sum to the full history. Negative windows retain completed batch outcomes and have zero contained successes.

**154 leaf-only drops:** same netem handle `5:`, parent `1:4`, same native Pod/container/netns lifetime, 0→154. Both raw command outputs and hashes are selected; parent counters are not added.

Six post-exit status snapshots pass the two-serial-fresh-empty gate for three voters. All five owned container lifetimes exited. Namespace UID `463e9e91-10f0-44a3-a218-da4625f2a014` and both owned node directories are absent; eight historical namespace UIDs remain unchanged.

Retained failures: root ancillary `len(integer windows)` reader before final audits; corrected preflight; initial publication destination collision before any copying (retained workload config now has its own path). No fixture or validator rerun.

Selection follows CRC exact-copy mechanics: original absolute source, relative publication destination, bytes and SHA-256. Complete process/Chaos histories, witnesses, reports, window/effect records, selected exact owned cleanup/leaf command output and source/build/image bindings are included. Binaries, raw WAL/store archives and host-wide process listings are excluded. External manifests reference additional retained local inputs; this selected evidence bundle is not a self-contained runtime archive.

One-host volatile-tmpfs correctness only; no performance/cross-host/power-loss claim. Exact-tuple TCP reset is explicitly non-Chaos. Full21 replacement, inter-voter partial loss/storage stalls and full Rust/formal refinement remain separate.
