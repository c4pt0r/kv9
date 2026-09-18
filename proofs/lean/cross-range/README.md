# Cross-range model

Finite-trace projection of one span crossing a split boundary, served in
range-sized chunks. Partition-directory exactness and each group's own
linearizable read are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `publishPartition` | the committed split directory (split-publication model) |
| `serveLow`/`serveHigh` (partition required; key order) | the chunked walk in `raw_scan_paged`/`raw_delete_range`: resolve the cursor's unsealed covering group, clamp the chunk to its range end, serve under that group's own authorization, continue from the boundary (`crates/server/src/runtime.rs`) |
| foreign-leader pauses | scan pages carry an explicit `resume_from` cursor (even over an empty page); delete-range receipts do the same with committed chunks staying committed |
| no skip/duplicate/atomic-span step | the partition covers exactly once; the span is never one snapshot — the documented per-chunk contract |

Eleven theorems: safety initialization/preservation/reachability, chunks
walk in key order, chunks require the partition, no key is skipped, no
key is duplicated, the span is never one snapshot, a chunk is permanent,
a partition alone serves nothing, and the span completes across the
boundary. The runner checks five semantic mutation controls and two
proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about cross-range transactions, automatic
triggers and scaling.
