# Raft method isolation and queue bounds

`Wire.lean` proves 15 statements about the explicit transport model. Run:

```sh
python3 scripts/prove-group-wire.py --lean /path/to/lean --output /fresh/output
```

The accepted compiler is Lean 4.33.1; the result records its executable hash.
The runner checks source hashes, compiles with warnings as errors, audits every
theorem's axioms, rejects six semantic defects, and rejects two proof-policy
violations. The observed dependencies are standard Lean `propext`,
`Classical.choice` and `Quot.sound`; no project axiom or proof hole is allowed.

The 2026-09-16 source-pin review raises the internal package from V2 to V3 to
exclude pre-range writers for both metadata and data traffic. There is still
no fallback, and each generation uses distinct metadata/data methods. The
same-generation old-method fixture continues to test method isolation; the
separate production-binary V2-to-V3 upgrade gate tests the new version floor.
This strengthens admission without changing the model's method/queue rules.

## Model and implementation mapping

| Model | Implementation | Boundary |
| --- | --- | --- |
| `method` | `StreamClass::for_region`, `GrpcTransport::enqueue`, `peer_session` | Wire ID 0 selects metadata; ID 1 is reserved; IDs greater than 1 select data. The selected method is fixed for the worker and reselected from its class on every session. |
| `receive` | Separate protobuf RPC paths; `RaftGrpcService::receive_raft` preflight | Old dispatchers have no `BatchDataRaft`. Modern receivers refuse the wrong domain before admitting any envelope in that batch. |
| `deliver` generation equality | `PeerDestination` allocation identity, `receive_for_destination`, `coalesce_queued` | Queued messages retain an `Arc`; its allocation cannot be reused while retained. Route replacement cancels both class sessions. Already transmitted network bytes are not recalled. |
| `arbitrary_reconnections_safe` | No version cache or fallback in `peer_session` | The per-attempt invariant holds for either endpoint version, independently of previous successful sessions. Instantiate it at each route generation. |
| `admitBatch` | The whole-batch method/domain preflight | This models domain rejection, not an atomic Raft batch or rollback of earlier batches. |
| `Queues`, `enqueue`, `Step`, `Reachable` | Separate `PeerRoute.sender` / `data_sender`, each a bounded Tokio channel | Count bounds and metadata queue reservation only. Draining/dropping cannot increase a count. Retry does not merge the queues. |

The main theorem permits only exact-group delivery or a drop for any endpoint
version and any queued/current generation. A separate theorem exhibits the
counterexample if data uses the legacy method: the legacy receiver consumes it
as metadata. The queue invariant holds after arbitrary sends, drains and
retries, and a full data queue does not consume metadata admission capacity.

## Explicit premises and exclusions

This is a checked mathematical model with source pins and an inspected mapping,
not verified extraction of Rust, tonic, Tokio, HTTP/2, or the network stack.
It assumes a compliant sender, honest method dispatch, and the existing Raft
crash/partition fault model. Authentication/root checks remain necessary; this
does not prove Byzantine tolerance. Tokio channel capacity and Rust ownership
are implementation premises, tested at their boundaries.

The model does not prove eventual delivery, fair scheduling, total node memory,
bounded socket buffers, CPU or disk isolation, creation/activation authority,
membership changes, public range epochs, split safety or throughput. The two
4096-message queues also have separate stream buffers; group inbox limits and
future node-wide byte budgets are separate obligations. Raft still tolerates
best-effort message loss and must not acknowledge uncommitted writes.

The [validation report](../../../docs/GROUP-WIRE-FENCING.md) distinguishes the
real loopback HTTP/2 fixtures from actual old binaries and actual Chaos Mesh.
