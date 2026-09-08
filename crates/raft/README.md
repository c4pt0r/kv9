# kv9-raft

`RaftPeer` adapts raft-rs 0.7 `RawNode`/`Ready`. `DiskRaftStorage` persists entries,
HardState and the applied ConfState position before messages leave the node.
`NodeDriver` owns the only Ready consumer, routes membership changes, establishes
quorum ReadIndex barriers and correlates proposals by exact `(term,index)`.

Ordered state-machine apply can only use `ApplyStore::write_applied`: data and its
position persist atomically. Recovery accepts the engine's durable pair after the
runtime checks it against committed Raft history. Epoch fences and manifest generation
CAS execute in ordered apply; remote I/O remains in the server's background worker.

`SingleNodeRaft` and in-process peers are test tools. The production runtime currently
hosts one group for catalog and Raw KV. Multiple groups, snapshot installation and
Raft log truncation remain roadmap work. See [roadmap](../../docs/ROADMAP.md).
