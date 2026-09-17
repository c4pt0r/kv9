# Runtime-adoption model

Finite-trace projection of one destination group generation crossing from
installation to runtime. Selection atomicity and whole-pair image
verification are premises from the joint-install model, cut exactness from
the source-capture model, learner membership from the attach model.

| Model element | Implementation |
| --- | --- |
| `install` (open path, exact positive cut) | `JointInstaller::install` selecting a generation (`crates/raft/src/snapshot_install.rs`) |
| `observeEngine` (pre-adoption only) | engine watermark recovery before runtime start (`MemStateMachine::with_engine`) |
| `adopt` (selected, engine ≥ cut; one-way close; driver := cut) | `adopt_for_runtime` marker publication + `RaftPeer::with_installed_storage` + `NodeDriver::with_installed_base` |
| `restart` (position forgotten) | driver restart: `driver_applied` starts `None` |
| `readopt` (marker required; exact cut restored) | marker-checked re-adoption in `adopt_for_runtime` |
| `tail` (adopted only; MsgAppend growth) | leader retained-log replication to the learner |
| no serving/promotion/network-snapshot step | reads refuse at the learner; no promotion path; `MsgSnapshot` receive drop |

Fourteen theorems: safety initialization/preservation/reachability, adoption
requires a selected generation and closes installation permanently, the
engine never sits behind an adopted cut, every restored driver position lies
between the cut and the engine, no serving and no promotion, network
snapshots never install, adoption is permanent, restart forgets the position
and re-adoption restores exactly the cut, a selected generation alone starts
nothing, and the engine never regresses after adoption. The runner checks
eight semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about promotion, source release, split and scaling.
