# Raft append synchronization batching

Tracking: #20 and #9. This first write-path candidate synchronizes the entry
slice supplied to one `DiskRaftStorage::append` call once. It retains the current
frame format, Ready ordering and engine durability contract.

## Persistence boundary and refinement argument

The writer mutex spans frame encoding, ordered writes, synchronization and
memory publication. Each frame is written using the existing length/checksum
encoding. Temporary encoding allocation remains per record; no second complete
batch buffer, delayed aggregator or additional queue is introduced. The slice
size remains the caller's responsibility, so this change is not a global queue
or process-memory bound.

The successful sequence is:

1. Append the existing encoded frames in entry order without changing the Raft
   storage view. Empty slices skip both writes and synchronization.
2. Synchronize the file once. A successful synchronization covers the complete
   byte prefix containing every frame written by this call.
3. Apply the complete supplied entry slice to the in-memory Raft storage view
   and return success. The caller still performs any original Ready HardState
   and LightReady commit persistence before publishing the cycle.

Assuming the existing filesystem contract (successful synchronization survives
the modeled crash and replay accepts complete valid frames), success implies
that replay can reconstruct every entry exposed by this append. A failure at
encoding, any frame write, synchronization or memory update fences the writer.
No append success is returned. Before successful synchronization, the memory
view is unchanged. A crash can retain an unacknowledged valid frame prefix; an
error reported after the synchronization effect can retain the complete batch.
Neither outcome permits the failed Ready cycle to publish an acknowledgement.

This maps to the existing `RPPersistEntries` success action and
`RPFailBefore`/`RPFailAfter` prefix outcomes in
[Ready publication](READY-PUBLICATION.md). That model already permits failed
entry-batch prefixes. Its invariant and transitions are unchanged; the mapping
still requires upstream Raft to supply valid entries and never overwrite a
committed prefix. This written source refinement is not a mechanically verified
Rust proof. Exact-candidate proof/refinement acceptance remains open.

HardState and configuration records retain their existing synchronous writes.
Engine application still synchronizes each applied command. Low-concurrency
one-entry appends therefore retain their original synchronization count, and
this increment alone does not promise Redis-class write throughput. Combining
Ready persistence or engine application requires its own explicit boundary and
failure analysis. No indispensable node or new coordination service is added.

## Local validation and outstanding acceptance

The Raft package passes 140 unit tests, 19 integration tests and 12 doctests.
New tests check one synchronization for three ordered frames, zero I/O for an
empty slice, and acknowledged batch survival after loss of unsynchronized bytes.
The actual storage implementation also passes 396 modeled write/synchronization
error and crash combinations: EIO/ENOSPC before and after each operation, short
writes, loss/retention/seeded survival of unsynchronized bytes, fencing without
further I/O, exact durable-prefix recovery and a second immediate crash after
recovery. These are deterministic `ModelFs` executions, not physical power loss
or actual Chaos Mesh.

Warning-denying all-target Raft Clippy passed. Applicable premature-publication
and omitted-sync implementation controls, exact-candidate process/MinIO/Chaos
histories, the checked refinement record, and paired release throughput/batch
observations remain required before acceptance. This candidate does not close
#20 or any original roadmap item. Daily verification remains local.
