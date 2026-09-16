# Protocol snapshot persistence model

The model checks one forward installation attempt for arbitrary old and new
protocol images and arbitrary finite failure/recovery traces. An image abstracts
the entire snapshot cut/term, persistent election term/vote, full configuration
and opaque payload identity. Eleven theorem statements establish:

- Inductive initial, step and trace invariants.
- Recovery selects one complete old or new protocol tuple.
- A successful storage return requires the complete new record to be durable.
- A failed append/sync cannot publish an in-memory installation or success.
- Recovery preserves the prior election term and an existing vote in that term.
- A compacted protocol base cannot start today's peer constructor.
- An uninstalled incoming snapshot leaves protocol state unchanged.
- A supplied snapshot remains byte-identity-equivalent at its actual cut; a
  newer requested cut is refused instead of relabeling the image.

`python3 scripts/prove-protocol-snapshot.py --lean /path/to/lean --output /fresh/path`
compiles the proofs, checks the complete named theorem dependency inventory,
rejects omitted proofs/custom axioms, verifies reviewed source hashes and runs
nine semantic defect controls. Lean's standard logical dependencies are the
only permitted axioms. Source pins establish correspondence inputs, not verified
Rust extraction or parser correctness.

| Model transition/predicate | Concrete correspondence |
| --- | --- |
| `Forward` | `validate_transition`: forward committed cut, monotone persistent term, existing same-term vote unchanged; structural snapshot/HardState checks |
| `begin` | One exclusive protocol writer after input validation; no local publication |
| `sync` | One complete `REC_SNAPSHOT` frame containing both snapshot and HardState, followed by successful `sync_data` |
| `publish` | Memory snapshot, HardState, configuration replay guard and selected snapshot are updated only after that sync |
| `failWrite`, `failSync` | I/O errors fence the writer; recovery may retain a complete unacknowledged frame |
| `crash`, `recover` | Existing checksummed record recovery, structural/transition revalidation and recovery synchronization before returning the store |
| `mayStart`, `receiveSnapshot` | Constructor refuses `first_index != 1`; receive drops MsgSnapshot before raft-rs can restore its membership/commit; Ready has a separate terminal guard |
| `serve` | Disk storage returns the exact retained snapshot only when its cut satisfies the request |

The old-or-new framing premise comes from the existing checksummed append-log
contract: a torn/checksum-invalid final frame is discarded; a complete frame is
parsed atomically; a checksum-valid invalid record refuses recovery. No framing
checksum collision, arbitrary disk rewriting, loss of acknowledged fsync data,
or concurrent owner of the same files is included in ordinary process crashes.
The deterministic filesystem tests execute the actual serializer and ordering
at every I/O cut and every frame truncation boundary. Actual media power-loss
behavior requires separate qualification.

This is the protocol-storage half of S05/D03. The opaque payload does not prove
root/range identity, committed source publication, engine-image correctness,
remote admission or retention ownership. No node may start from this base yet.
An engine installation journal, validated destination authority and object pins
must be composed before enabling snapshot reception, learner attachment or log
reclamation. The model does not prove complete Raft, membership liveness, online
migration, automatic split, horizontal scaling or cross-host fault independence.
