# Reuse the published WAL directory during rotation

Status: experimental source candidate [`483b8c3`](https://github.com/c4pt0r/kv9/blob/483b8c3629b033734f5d7a2b8653a1352304a4b5/docs/WRITE-PUBLISHED-DIRECTORY.md) based on selected CRC `bd42e60`.
It does not combine the FNV writer or receipt tail-hint experiments. No database
performance result or production promotion is claimed for this candidate.

## Problem and behavior

`WalSegment::create` previously synchronized the segment file and then every
canonical ancestor directory through `/`. `SegmentedWal::rotate` called that
public constructor for every successor. A WAL under `/dev/shm` therefore still
performed a directory fsync on the host root during each rotation. This is a
source-level observation; it does not establish the cause of any FNV tail.

A live writer now retains a private `PublishedSegmentDirectory` containing the
canonical parent path and an open directory descriptor. Initial creation and
active-file recovery still synchronize the complete ancestor chain. Only a
successful seal transfers the capability to successor creation. On Unix,
successor creation checks the canonical parent and held device/inode before
creation and before directory synchronization, creates the file exclusively,
synchronizes the complete header, and synchronizes the immediate directory.
Platforms without the Unix identity check keep full namespace synchronization.

The enclosing stream still fences before retiring its writer and installs the
successor only after complete topology publication. The topology file sync,
rename, and topology-parent sync are unchanged. No checksum bytes, append sync,
Raft quorum, application order, or response condition changes. The capability
cannot be reconstructed from a decoded closed-segment descriptor or copied by
clients. It retains one additional directory descriptor per active writer.

## Conditional proof and limits

The namespace edges are pairs binding names to object identities. Let `A` be
the complete ancestor-edge set and `D` the durable edge set. Initial full sync
establishes `A subset D`. If the owner preserves these exact ancestor edges,
then adding the new segment edge `e` to `D` has the same result as adding both
`A` and `e`: `D union {e} = D union A union {e}`. A successful file sync supplies
the separate file-content premise. The proof preserves existing edges and the
capability across arbitrary subsequent rotations, and permits reclaiming child
edges outside `A`.

The [TLA+/TLAPS module](https://github.com/c4pt0r/kv9/blob/483b8c3629b033734f5d7a2b8653a1352304a4b5/proofs/tlaps/published_directory/PublishedDirectoryProof.tla)
contains 12 conditional statements, discharged by 24 fresh obligations. Four
explicit counterexamples demonstrate missing capability, missing parent sync,
missing file sync, and changed ancestor identity. Semantic auditing rejects an
omitted proof and an extra false axiom; three output controls reject incomplete
proof results. The source contract binds both changed runtime files.

This is a refinement of the namespace premise in `WTCreate` in the existing
[WAL topology protocol](WAL-TOPOLOGY-PROOF.md). It does not re-prove the complete
Rust implementation, Raft, the filesystem, or power-loss behavior. The filesystem
must provide the stated successful-file-sync and directory-sync guarantees.
A file sync alone does not guarantee that its directory entry is durable.
See the [Linux fsync documentation](https://man7.org/linux/man-pages/man2/fsync.2.html).

Exclusive store ownership must preserve the directory and its ancestor names.
The identity checks detect an observed path/inode replacement; they do not
serialize against concurrent external rename, mount, unmount, or namespace
mutation. An in-memory capability does not survive process failure. Recovery
always establishes it again through full synchronization. These premises are
explicit; removing an ancestor edge invalidates the optimization's proof.

## Qualification and next gate

Local source qualification passes **793 tests/doctests (23 existing ignored)**,
formatting and workspace/all-target Clippy with warnings denied. The retained
BuildCache transaction explicitly invalidates first-party development artifacts;
all compiler inputs and the complete source inventory remain unchanged during
qualification. Four new unit tests cover capability transfer through
rotation/recovery, a different parent, inode replacement at the same path, and
exclusive file creation.

The Linux production-API probe passes **five cases and four refusal controls**.
The baseline checks two rotations. Actual traces require full ancestor sync on
initial creation and each recovery, and only immediate-segment-parent plus
topology-parent directory sync during each successful rotation. They also require
successor-file sync before segment-parent sync and topology publication. EIO
injected before/after the successor file sync and before/after its parent sync
must fence the writer and preserve all previously acknowledged records through
recovery. No production fault hooks are compiled into the library/server.

The four controls reject an absent shim, a wrong injection target, an omitted
initial ancestor sync, and an omitted successor-parent sync. Both omission
controls explicitly return success without performing the selected real sync;
the acceptance reader must reject them at the corresponding trace predicate.
All nine probe lifetimes and the shim compiler exit; original data and traces
remain retained. These are syscall-result tests, not physical power cuts or
actual Chaos Mesh. The runner also checks that source and the retained probe
hash remain unchanged.

Local terminal records: proof `ef45a2/0`, source `98335/7c7935/0`, syscall gate
`16016/2057de/0`. The retained probe SHA256 is
`c0e86eba0d46b7aafb36d4275bfa27c9f4fca5143fe67a17e3c538ab494c9087`.
The first proof reporter counted six output controls although one module ran
three; the corrected reporter derives the count from the shared helper. Its
original result is preserved separately. Both runs proved the same 24 obligations.

Actual Chaos Mesh, default release recovery, and a matched throughput/latency
comparison remain separate required runtime gates. Keep CRC main selected.
The latest accepted performance figures remain unchanged; no hosted CI ran.

[Portable original source, proof and syscall evidence](https://github.com/c4pt0r/kv9/blob/483b8c3629b033734f5d7a2b8653a1352304a4b5/docs/published-directory-source-v1/README.md) contains 175 members / 8,553,269 decoded bytes in a 646,232-byte archive. Independent member verification passes. The [retained FNV tail analysis](WRITE-FNV-TAIL-ANALYSIS.md) explains the investigation context without attributing causality.
