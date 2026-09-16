# Durable group preparation model

`Preparation.lean` proves nine statements about the local preparation protocol,
including inductive safety for arbitrary finite traces, exact binding and two
durable stores before a successful observation, no initialization after durable
readiness, failure fencing, monotonic readiness and discarded observations on
restart. The checker audits theorem axioms and runs four single-defect model
controls plus proof-hole/custom-axiom rejection controls.

Run from the repository with a pinned Lean binary and a fresh output directory:

```sh
python3 scripts/prove-group-preparation.py --lean /path/to/lean --output /path/to/new-evidence
```

This is a protocol-component proof with a reviewed source mapping. The source
hash contract detects drift; it is **not verified extraction of the Rust code**.
It does not prove data-group activation, Raft itself, range publication, split,
membership change, scheduler fairness, liveness under permanent I/O failure or
horizontal scaling. Those remain the enclosing D01/P2 obligations.

| Model | Concrete implementation/refinement premise |
| --- | --- |
| Fixed `authority` | Exact immutable creation descriptor: certified root, operation/task/group IDs and initial node/store-incarnation bindings. The catalog planner lock, prior ordered barrier and same-term commit preserve the existing metadata serialization contract. |
| `intent` | `RegionManager::open_local` accepts `CommittedCreation`, checks local root/node/incarnation and synchronizes the intent record before opening either log. `committed_creation` reads a fresh applied-engine snapshot, never a caller's planning overlay. |
| `raft` | Successful durable `DiskRaftStorage::open` during IntentDurable, with validated initial configuration and no voting history. StorageReady uses `recover`, which cannot create a missing log. |
| `engine` | Positioned WAL recovery, rejection of checkpoints/data/applied history for an unstarted group, followed by the existing durable segmented-layout publication. |
| `ready` | StorageReady record publication only after both opens/validation succeed; file sync, atomic rename and directory/ancestor sync complete before success. |
| `report` | Insert the owned prepared handles and return `GroupPreparation` only after all previous steps succeed. An idempotent in-process return refers to the same durable observation. |
| `fail` | Any ambiguous local preparation error stores a failed slot; retries cannot initialize or return success until a new manager recovers it. |
| `restart` | Process observations/handles are discarded. Synchronize the recovered record and validate exact metadata/local identity and both stores before producing another observation. A missing first record can be reconstructed only from the immutable metadata intent and an otherwise unused directory. |

The durable-step model treats a crash between publications as retaining the
last synchronized state. A visible but not yet directory-synchronized record
is provisional: concrete recovery resynchronizes it and validates its stores
before admitting the corresponding model transition. No network/serving action
exists in the preparation model. This is why an interrupted initial preparation can
initialize an empty log, while a StorageReady record never may.

Explicit premises: the existing WAL recovery and fsync/rename/directory-sync
contracts, non-Byzantine local storage under exclusive ownership, the existing
metadata committed-prefix/term-fencing proof, correct codec/digest binding
(collision resistance where a digest is used), and successful Rust control
flow matching the table. Arbitrary filesystem replacement or a medium losing
acknowledged fsync data is not modeled as an ordinary process crash. Local
fault-cut tests validate ordering at 16 boundaries; they do not prove physical
power-loss behavior. Actual Chaos Mesh acceptance remains separate.

The subsequent [activation model](../group-activation/README.md) adds runtime
ownership. For this preparation projection, Active retains StorageReady's
identity and durable-store prerequisites; activation and ordinary execution
are stuttering steps. Preparation failure is distinct from a later driver's
fatal state: `GroupPreparation` is a durable preparation observation, never a
health or serving receipt. Active recovery permits committed data/voting
history under its separate validator and never reinitializes a store. Current
source pins are refreshed for this inspected refinement, and all nine original
theorems/controls are rerun. The historical evidence packet is unchanged.
