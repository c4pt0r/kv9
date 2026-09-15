# Published-directory rotation and recovery

On 2026-09-15 UTC, candidate `483b8c3` completed the actual Chaos Mesh
rotation supplement: selected WAL rotation on all three voters, leader
container-kill, same-store acknowledged-value recovery, and a second rotation
on all three voters after recovery. Independent full-history audit, archive
readback and exact cleanup pass. CRC main remains selected; this correctness
workload provides no new throughput or latency measurement.

The exact default server/image and 16 MiB segment target match the
[accepted 21-window baseline](WRITE-PUBLISHED-DIRECTORY-CHAOS.md).
Two finite single-worker bursts each use a fresh keyspace, 64 keys,
8,192-byte values and batches of 64. Both complete 56-call histories pass
the original atomic-history validator with every call successful. Combined
traffic contains 90 BatchPut and ten BatchGet calls: 5,760 acknowledged write
items and 47,185,920 input value bytes, excluding initialization writes.

| Checkpoint | Every voter's selected topology | Acknowledged write fence |
| --- | --- | --- |
| Before the fault | Generation 2; closed segment 1, active segment 2 | Term 1, index 51 |
| After same-store recovery | Same streams and generation 2 topology | Term 1, index 51 |
| After the second burst | Generation 3; closed segments 1 and 2, active segment 3 | Term 2, index 101 |

The selected topology checksum, stream identity, sequence, predecessor and
physical segment headers are independently checked; filenames and offered
bytes alone are insufficient. Chaos Mesh killed the observed leader's exact
container, which exited 137. The replacement kept its Pod UID, PVC and store
lifecycle; the old server process was observed absent. Complete public reads
recover the 65-key dataset, including the empty sentinel, after restart.
Final reads verify both keyspaces, each with 524,288 value bytes. Five drains
require two new status publications on every voter, empty queues, healthy
serving state, agreed leadership and fully applied committed progress.

The successful runtime is `91616/8d6682/0`, independent audit
`15025/2cf21d/0`, archive/readback `98295/355350/0`, and cleanup
`99235/092c9f/0`. All five remaining containers and their process trees exit;
both native workload children had already exited. The previously killed
leader is accounted for separately. The namespace is absent and all eight
historical namespace UIDs are unchanged.

Two earlier attempts remain failed. The first lacked the installed Chaos
controller's required namespace annotation, `chaos-mesh.org/inject=enabled`,
and failed before injection. Its [original prefix evidence](published-directory-rotation-prefix-v1/README.md)
is preserved. The second injected the fault but hit a test-script error:
command-only applied position `(1,51)` was incorrectly required to equal
unified driver/commit position `(2,52)` after an election no-op. A separate
read-only diagnosis recovered all acknowledged values; it did not accept
that incomplete attempt. Its failed result and subsequent exact cleanup are
preserved with the successful attempt.

The corrected checker follows the existing driver's documented semantics:
unified driver-applied index must equal committed index, while every voter's
command watermark must cover the maximum successful native write receipt
derived independently from the complete history. Coherent term/index pairs,
queue, freshness, identity and deadline requirements remain. No extra write
forces the no-op gap closed. Five focused controls pass (`7c096d/0`), including
rejection of unapplied commits and progress below the acknowledged prefix.
No production source or consistency rule changed to make the test pass.

[Portable original histories, failure records and verifier](published-directory-rotation-v1/README.md)
retain both later attempts. The successful archive contains 1,314 members /
227,025,332 decoded bytes; the second failed attempt contains 1,324 members /
115,917,336 decoded bytes. Every original member was independently read back
before cleanup. The archives include selected topology and segment headers,
but are not complete raw PVC WAL/SST or executable backups.

The subsequent [complete matched write screen](WRITE-PUBLISHED-DIRECTORY-PERFORMANCE.md)
now passes all eight smokes and sixteen timed cohorts. Batch throughput/tail
improve, while loaded point throughput regresses slightly; CRC stays selected.
[Local capacity recovery](LOCAL-CAPACITY-RECOVERY.md) enabled the campaign;
future runs still require fresh space guards. Dynamic
multi-Raft, routing, membership and automatic splits remain on the subsequent
route. This checkpoint closes the rotation supplement, not an entire original
industrial roadmap work package. All work ran locally; no hosted CI was dispatched.
