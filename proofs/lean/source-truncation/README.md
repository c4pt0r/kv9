# Source-truncation model

Finite-trace projection of one source group's log-prefix compaction under
committed authority. Install-evidence settlement and storage/persistence
atomicity are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commitEvidence` | committed kind-104 evidence row with its exact cut |
| `decide` (evidence required, floor ≤ cut, one per operation) | `plan_truncation` kind-105 row + the server's Released-pin check (`crates/meta/src/data_groups/truncation.rs`) |
| `advance` | tracked-peer matched progress (raft `prs()`) |
| `compact` (decision + every peer matched ≥ floor) | `RaftPeer::truncate_retained_log` → `DiskRaftStorage::compact_retained_prefix` (REC_COMPACTION record; tail kept) |
| `restart` (full log, or retained == the committed floor) | `RegionManager::start_group` compacted-base gate over `committed_truncations` |
| no unauthorized-restart or floor-regression step | the blanket refusal stays for any other compacted base |

Thirteen theorems: safety initialization/preservation/reachability, a
decision requires evidence below its cut, compaction never exceeds the
committed floor, no tracked peer is left behind the retained bound, a
compacted log implies the full committed chain, no unauthorized restart,
the retained bound never regresses, the floor is immutable once decided,
matched progress is monotone, a decision alone compacts nothing, and the
committed chain reaches compaction and an authorized restart. The runner
checks eight semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about physical disk reclamation, stranded-learner
recovery, promotion, removal, split and scaling.
