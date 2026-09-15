# Checkpoint publication during local recovery

C04 increment, 2026-09-15, tracking [#14](https://github.com/c4pt0r/kv9/issues/14)
under [#9](https://github.com/c4pt0r/kv9/issues/9).

Startup now requires evidence that the selected remote checkpoint actually won
manifest publication. Previously, it checked that the descriptor appeared in a
committed Raft proposal. A committed proposal can lose the generation CAS and
must not authorize recovery. A regression test reproduces that distinction with
the actual state machine and durable Raft/engine logs.

This is local recovery validation. Complete portable anchors, root/range and
destination binding, the replicated retention ledger, snapshot installation and
dynamic multi-Raft remain unfinished. C04 stays open.

The subsequent [historical base identity increment](CHECKPOINT-BASE-IDENTITY.md)
adds a runtime callback between verified SST restore and tail replay, and makes
upload scope come from the same frozen image. The publication protocol below
is unchanged; its source pins are revalidated for that composition.

## Implementation

`open_checkpoint_engine` owns the validator and the engine open. Callers cannot
supply observations or finalize a different engine. The existing store guard
remains held; startup has not constructed the Raft peer or exposed Serving.

1. The engine reports its actual selected checkpoint before object restoration.
   The validator checks canonical scope and obtains the full configuration at
   the exact committed image cut `c` using `configuration_at_committed`.
2. The existing remote restore verifies SST bytes and hashes. Only uncovered
   WAL records enter the publication observer.
3. An observed generation descriptor must have exactly one matching manifest
   pair in the same positioned atomic batch. The pair binds the generation and
   canonical predecessor/change identity. Its position `p` must be after `c`.
4. The retained committed Raft entry at that exact term/index must be a normal
   `ManifestChange` with matching region, predecessor, change identity,
   descriptor and watermark. Proposal presence alone cannot supply this batch.
5. After successful complete engine open, the durable applied position must
   cover `p`; the original descriptor and a compatible current pair must remain
   in the recovered state. Only then does the function return a private-field
   `RecoveredCheckpointPublication` alongside that engine.

The first actual winning publication is retained. An idempotent retry creates
no new witness. A legitimate later publication of the same descriptor at a
higher generation does not move the original result. Rewriting the same or an
older witnessed generation is refused.

The image predates its own publication (`c < p`). Existing engine checkpoint
reclamation therefore preserves the publication batch in the uncovered suffix.
This does not authorize protocol-log compaction: the configuration provider and
publication lookup still need retained committed history. Missing authority is
an error or unavailable result, never permission to guess from current state.

There is no new disk format, online apply operation, network round trip or
service. Every replica performs this local startup check. The observed manifest
bytes are encoded once; replay inspects batch mutations. Configuration lookup
can scan retained history under its writer lock, so this API remains off the
online read/write and periodic checkpoint paths. No new throughput claim is made.

## Proof and its premises

[CheckpointPublication.tla](../proofs/tla/checkpoint_publication/CheckpointPublication.tla)
has **9 theorem statements / 36 strict TLAPS obligations**. The deductive proof
is parameterized by an arbitrary positive finite committed prefix; TLC checks
two bounded instances independently.

| Model action/state | Source correspondence |
| --- | --- |
| `CPApply`, `cpWinners`, `cpWinnerGeneration` | Existing ordered generation CAS writes the pair, descriptor and position in one durable engine batch only on success |
| `CPBegin` | Observe the selected base and fix its committed configuration before restoring objects |
| `CPScan`, `CPMatches` | Inspect actual uncovered batches; bind a winning descriptor to the selected image and exact committed command term |
| `cpSeen` | Keep the first actual matching winner across retries or later generations |
| `CPComplete`, `CPGrant` | Complete engine open, then require final recovered history agreement before returning a publication |
| `CPFail` | Failed or partial recovery returns no publication capability |
| `CPRank` | Each actual scan/completion/grant/failure step decreases a natural-valued rank |

The invariant implies that every granted observation has an actual CAS winner,
correct image, generation, term and publication after the image cut, and comes
after complete recovery with final history agreement. A losing CAS creates no
witness. The proof also establishes first-winner stability and finite local scan
progress; it does not promise successful I/O, fair scheduling or a restart deadline.

Explicit premises are the existing immutable committed Raft prefix, correctly
ordered manifest apply and atomic intact WAL batches. The proof does not derive
those premises from arbitrary disk bytes, reprove Raft or mechanically refine
Rust execution. Root/store admission remains in the enclosing runtime; complete
range epochs, destination authority, object lifetime and installation binding
belong to the unfinished anchor layer. CRC-valid malicious file fabrication and
object-store power-loss durability are outside this model.

The reproducible runner checks strict/no-fingerprint proofs, semantic assumptions,
theorem inventory and source/tool hashes. Five actual counterexamples remove
winner membership, image agreement, term agreement, complete-open ordering or
final history agreement. Proof-hole, custom-axiom and unsafe-selection variants
are rejected separately.

## Local validation

- Engine **137**, Raft **262**, server **207** library tests pass; the two ignored
  tests in the combined run are reported separately. All-target Clippy with
  warnings denied and formatting pass.
- The new actual MinIO test passes explicitly with `--ignored`: both legacy and
  segmented layouts accept the winner and reject a committed losing descriptor.
- The default binary passes the three-node MinIO process E2E: **272 acknowledged
  filler writes** force physical WAL rotation and prefix reclamation, followed
  by leader failover/rejoin, full-cluster restart, checkpoint plus live-tail
  restart, deletion checks and a fresh successful write. Every node's logs show
  the new generation/cut/publication observation after restart.
- Three isolated Rust mutations compile and fail their exact selected test:
  accept a foreign scope, skip the atomic pair check, or replace the first
  winner with a later publication. An unchanged source copy passes the five
  component tests first; those tests overlap the Raft suite.
- The actual Chaos Mesh cell passes **296 successful serial operations**: one
  keyspace creation, 285 puts (280 force WAL rotation), two deletes and eight
  reads. An observed `PodChaos/container-kill` terminates voter 2 with exit 137.
  Its new container keeps the same Pod/PVC/store and reports generation **210**,
  image cut **499**, publication **500**. Post-restart reads agree with the
  acknowledged state and all three voters apply the fresh write. Independent
  replay of the complete serial history, fault/lifetime evidence and selected
  SST hashes passes. The owned namespace is removed; eight protected namespaces
  and the pre-existing fault are unchanged.

The Chaos cell uses the same pinned default binary as the process E2E:
`bf6099efb05d8143747b7994e3ffd72847fede1e1310757c3b3adca7a8f16c32`.
It keeps an 8 GiB free-space floor and the original preparation baseline across
retries. Maximum observed filesystem decrease is **978,882,560 bytes**, within
its separate 1 GiB cell budget. This is one single-host container-failure case,
not the historical full21 matrix, cross-host failure or power-loss acceptance.
See the [portable evidence](checkpoint-publication-v1/README.md) for exact
inputs, outcomes and retained original failures.

All execution is local. The isolated MinIO fixtures use bounded memory-backed
data storage because the host-backed fixture refuses PUT with `507
XMinioStorageFull`. They exercise real S3 calls and KV process recovery, not
object-store crash/power-loss persistence. This does not replace the original
full performance screen or change the selected CRC write implementation.

The original failed attempts are retained: Rust moved-value/private-method
compile errors, a TLA precedence error rejected by independent SANY despite an
earlier TLAPS result, a Python mutation literal syntax error, the host MinIO's
507 refusal, and a source-control build missing testing-feature dependencies.
The final proof includes the SANY-compatible parentheses. Source-control
compilation errors never count as successful fault detection.
Three original Chaos setup failures are also retained: an unsupported Python
file-opening argument, a provisioner deletion deadline conflicting with its
termination grace, and an empty CA trust store in the minimal image. All precede
client workload and fault injection. The accepted image adds the existing CA
bundle; the database binary is unchanged. The HTTP endpoint still constructs a
client with the platform TLS verifier, which requires that runtime input.
The published Chaos helper differs from the executed copy only in including a
selected-SST inventory file in its public evidence selector. Both source copies,
the one-line diff and the initial metadata-selector assertion failure are
retained. This metadata correction did not rerun or change the accepted workload.

## Reproduce

```sh
cargo test --offline -p kv9-engine -p kv9-raft -p kv9-server --lib
cargo clippy --offline -p kv9-engine -p kv9-raft -p kv9-server --all-targets -- -D warnings
cargo fmt --all -- --check
python3 -W error scripts/check-checkpoint-publication.py \
  --jar /path/to/tla2tools-v1.7.4.jar --tlapm /path/to/tlapm \
  --output /new/publication-proof
cargo build --offline -p kv9-raft --features testing --message-format=json \
  > /path/to/cargo-artifacts.jsonl
python3 -W error scripts/check-checkpoint-publication-source-controls.py \
  --artifacts /path/to/cargo-artifacts.jsonl --output /new/publication-controls
```

With an actual MinIO endpoint and credentials supplied through the documented
`KV9_OBJECT_STORE_*` environment variables, run the explicit ignored test and
`KV9_MINIO_EXTERNAL=1 KV9_BIN=/path/to/kv9 python3 scripts/minio-kv-e2e.py`.
Do not put credentials into evidence archives or command output.

`scripts/checkpoint-publication-chaos.py` is a lab-specific local acceptance
runner, with explicit Kind cluster, kubeconfig, protected namespace and retained
image identities. Supply the pinned `--binary`, `--binary-sha256`, matching
`--inputs` and a fresh `--output`; it uses the local public CA bundle. It creates
and removes only its owned fixture. `--prior-preparation` requires the retained
cleanup record and preserves the original capacity baseline. Adapt and review
the explicit environment identities before using it in another lab.

## Next implementation steps

1. Inspect root and ownership metadata from the restored image **before tail
   replay**, through an owned startup interface. The current observer sees the
   descriptor before restore and batches afterward; neither the descriptor nor
   the final, newer catalog alone proves the complete historical range/epoch at
   the image cut. Bind the certified root, historical range and transition chain
   to this publication and its checked configuration. The current remote path
   freezes the full engine under `META_REGION_0`; logical routing rows must not
   be mistaken for independently recoverable per-range Raft groups.
2. Implement the versioned, bounded portable anchor envelope separately from
   destination installation authority. Keep image, publication and configuration
   positions distinct. Decoding constructs data, never a serving/reclaim token.
   Missing required authority stays unavailable; contradictory authority is
   invalid. Cross-root, old-store, wrong-range, unsupported-capability and
   interrupted-install controls accompany the composition proof.
3. Connect the existing retention records to an outer ledger owned by replicated
   metadata. Acquire before publication, retain unknown attempts across restart,
   transfer ownership without a zero-owner window, and fence delayed deletion
   with permanent object-instance retirement. Inventory pending attempts,
   readers, snapshots, migration and backups before allowing any GC consumer.

Then qualify bounded snapshot capture and atomic installation before protocol
truncation and remote attachment. Automatic range splitting and dynamic
multi-Raft keep their original dependencies and fault gates. The full matched
write comparison remains a separate capacity-dependent task.
