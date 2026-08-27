# kv9-raft

The consensus abstraction (DESIGN §6.1). Each region is an independent Raft group
(multi-raft); the metadata plane's L0 bootstrap group and L1 meta-regions are Raft
groups too (DESIGN §5).

- `RaftGroup` trait — propose/commit/apply + leadership + role.
- `SingleNodeRaft` — a stub that "commits" immediately (single replica), so the
  skeleton compiles and M1 can run single-node. Real consensus arrives in M2.

The design note that the raft log **is** the WAL for the region memtable (DESIGN §6.2,
§6.4) is honored by the `region` crate, which drives apply and flush watermarks.
